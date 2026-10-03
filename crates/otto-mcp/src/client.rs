//! Outbound MCP client — Otto connecting *out* to a registered MCP server.
//!
//! Two transports:
//! - **stdio**: spawn `command args` with `env` (incl. keychain-resolved secrets),
//!   speak newline-delimited JSON-RPC 2.0 over the child's stdin/stdout.
//! - **http** (Streamable HTTP): `POST initialize` (capture `Mcp-Session-Id`) →
//!   `POST <op>` (carry the session id). The reqwest client is built with the
//!   **SSRF-validated IP pinned** (`.resolve`) so a DNS rebind can't redirect the
//!   connect — and the configured auth header — to loopback/metadata (design §14
//!   F9). `otto_netguard::redirect_policy()` guards redirect hops.
//!
//! **Sessions (SE-14).** `list_tools` / `call_tool` keep ONE initialized session
//! per client (the stdio child stays up; the HTTP client + pinned IP + session id
//! are reused), so a pooled client (see `McpService`) pays spawn + `initialize`
//! once instead of per call (1–3 s for an `npx` server). A failed or timed-out op
//! drops the session (the child is killed); the next op starts a fresh one. A
//! reused session is retried once, fresh, ONLY when the request provably never
//! reached the server (write failed / stream already closed before we wrote) —
//! a tool call is never replayed after the server may have run it.
//!
//! **Concurrency (perf2/10-mcp R3).** A client parks up to [`SESSION_SLOTS`]
//! sessions. An op takes an idle parked session; when every opened session is
//! busy it waits up to [`BUSY_WAIT`] for one (most ops are far shorter than a
//! spawn), then opens a second parked session in a free slot, and only when
//! all slots are busy past the wait does it fall back to a one-shot child.
//! Two concurrent `gateway_call`s to one `npx` server used to pay a fresh
//! 1–3 s spawn for the second; now it reuses the first session or the second
//! slot. `health` is always one-shot (it probes that the server can START).
//! Children are `kill_on_drop`.
//!
//! Hard caps mirror the inward server: 20 s per op, 1 MiB body. Redaction/row-cap
//! of results happens in the service layer.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const PROTOCOL_VERSION: &str = "2024-11-05";
const OP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Parked (reusable) sessions per client: a second one serves sustained
/// two-way parallelism without a spawn per call; more would only keep idle
/// children alive.
const SESSION_SLOTS: usize = 2;
/// How long an op waits for a busy parked session before opening another.
const BUSY_WAIT: Duration = Duration::from_millis(250);

type SlotGuard<'a> = tokio::sync::MutexGuard<'a, Option<Live>>;

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

/// A spawned stdio server's handles (boxed: far bigger than the HTTP arm).
struct StdioIo {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
}

/// An initialized, reusable session.
enum Live {
    Stdio {
        io: Box<StdioIo>,
        next_id: i64,
    },
    Http {
        client: reqwest::Client,
        session_id: Option<String>,
        next_id: i64,
    },
}

/// Why an op on a live session failed — decides whether a fresh retry is safe.
enum OpError {
    /// The request never reached the server (write failed, or the stream was
    /// already closed before anything was written): safe to retry fresh.
    NotDelivered(String),
    /// The server may have seen the request: never replay it.
    Failed(String),
}

pub struct McpClient {
    transport: Transport,
    live: [tokio::sync::Mutex<Option<Live>>; SESSION_SLOTS],
}

impl McpClient {
    pub fn new(transport: Transport) -> Self {
        Self {
            transport,
            live: std::array::from_fn(|_| tokio::sync::Mutex::new(None)),
        }
    }

    /// True while an initialized session is parked on this client (tests /
    /// pool diagnostics). A busy slot counts as live.
    pub fn has_live_session(&self) -> bool {
        self.live
            .iter()
            .any(|s| s.try_lock().map(|g| g.is_some()).unwrap_or(true))
    }

    /// The slot an op runs on, or `None` ⇒ run it on a one-shot session.
    /// Order: an idle parked session → (nothing in flight) the first free slot
    /// → wait up to [`BUSY_WAIT`] for a busy session → a free slot (a second
    /// parked session) → `None`.
    async fn checkout(&self) -> Option<SlotGuard<'_>> {
        let mut free: Option<SlotGuard<'_>> = None;
        let mut busy: Vec<usize> = Vec::with_capacity(SESSION_SLOTS);
        for (i, slot) in self.live.iter().enumerate() {
            match slot.try_lock() {
                Ok(g) if g.is_some() => return Some(g),
                Ok(g) => {
                    if free.is_none() {
                        free = Some(g);
                    }
                }
                Err(_) => busy.push(i),
            }
        }
        if busy.is_empty() {
            return free;
        }
        // A session is mid-op. Ops are usually far shorter than a spawn, so
        // wait briefly for it rather than start another server process.
        let waited = match busy.as_slice() {
            [a] => tokio::time::timeout(BUSY_WAIT, self.live[*a].lock())
                .await
                .ok(),
            [a, b, ..] => tokio::time::timeout(BUSY_WAIT, async {
                tokio::select! {
                    g = self.live[*a].lock() => g,
                    g = self.live[*b].lock() => g,
                }
            })
            .await
            .ok(),
            [] => None,
        };
        waited.or(free)
    }

    /// `tools/list` → the advertised tool objects.
    pub async fn list_tools(&self) -> Result<Vec<Value>, String> {
        let result = self
            .op(json!({"jsonrpc":"2.0","method":"tools/list"}))
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
            "jsonrpc":"2.0","method":"tools/call",
            "params": {"name": name, "arguments": args}
        });
        let result = self.op(req).await?;
        let bytes = serde_json::to_vec(&result).map(|v| v.len()).unwrap_or(0);
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(CallResult {
            content: result,
            is_error,
            bytes,
        })
    }

    /// Health probe = an `initialize` round-trip on a ONE-SHOT session (it must
    /// prove the server can start, not that a parked session is still up).
    pub async fn health(&self) -> Result<(), String> {
        self.open().await.map(drop)
    }

    /// Run one post-initialize op, returning its `result`. Uses (and keeps) a
    /// parked session ([`Self::checkout`]); only when every slot stays busy
    /// past [`BUSY_WAIT`] does this op run on a one-shot session.
    async fn op(&self, request: Value) -> Result<Value, String> {
        let Some(mut guard) = self.checkout().await else {
            return self.one_shot(request).await;
        };
        let reused = guard.is_some();
        if guard.is_none() {
            *guard = Some(self.open().await?);
        }
        let res = {
            let live = guard.as_mut().expect("session just opened");
            tokio::time::timeout(OP_TIMEOUT, run_on(live, &self.transport, request.clone())).await
        };
        match res {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(OpError::NotDelivered(e))) if reused => {
                // The parked session died while idle (child exited, HTTP session
                // expired). Nothing reached the server: one fresh attempt.
                *guard = None;
                let mut fresh = self.open().await?;
                let out =
                    tokio::time::timeout(OP_TIMEOUT, run_on(&mut fresh, &self.transport, request))
                        .await
                        .map_err(|_| timeout_msg())
                        .and_then(|r| {
                            r.map_err(|e| match e {
                                OpError::NotDelivered(m) | OpError::Failed(m) => m,
                            })
                        });
                if out.is_ok() {
                    *guard = Some(fresh);
                }
                out.map_err(|m| format!("{m} (after stale session: {e})"))
            }
            Ok(Err(OpError::NotDelivered(e) | OpError::Failed(e))) => {
                *guard = None; // drop (and kill) the broken session
                Err(e)
            }
            Err(_) => {
                *guard = None;
                Err(timeout_msg())
            }
        }
    }

    /// A throwaway session for one op (the parked one is busy).
    async fn one_shot(&self, request: Value) -> Result<Value, String> {
        let mut live = self.open().await?;
        tokio::time::timeout(OP_TIMEOUT, run_on(&mut live, &self.transport, request))
            .await
            .map_err(|_| timeout_msg())?
            .map_err(|e| match e {
                OpError::NotDelivered(m) | OpError::Failed(m) => m,
            })
    }

    /// Spawn / connect and complete the `initialize` handshake.
    async fn open(&self) -> Result<Live, String> {
        match &self.transport {
            Transport::Stdio { command, args, env } => {
                tokio::time::timeout(OP_TIMEOUT, open_stdio(command, args, env))
                    .await
                    .map_err(|_| timeout_msg())?
            }
            Transport::Http { url, headers } => {
                tokio::time::timeout(OP_TIMEOUT, open_http(url, headers))
                    .await
                    .map_err(|_| timeout_msg())?
            }
        }
    }
}

fn timeout_msg() -> String {
    format!("op timed out after {}s", OP_TIMEOUT.as_secs())
}

fn init_request() -> Value {
    json!({
        "jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{"protocolVersion":PROTOCOL_VERSION,"capabilities":{},
                  "clientInfo":{"name":"otto-control-plane","version":"0.1.0"}}
    })
}

async fn open_stdio(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<Live, String> {
    let mut child = Command::new(command)
        .args(args)
        .envs(env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn '{command}': {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no child stdin")?;
    let stdout = child.stdout.take().ok_or("no child stdout")?;
    let mut reader = BufReader::new(stdout);
    write_line(&mut stdin, &init_request()).await?;
    let _ = read_until_id(&mut reader, 1).await?;
    write_line(
        &mut stdin,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await?;
    Ok(Live::Stdio {
        io: Box::new(StdioIo {
            child,
            stdin,
            reader,
        }),
        next_id: 2,
    })
}

/// SSRF-validate the URL, pin the vetted IP, build the client and initialize.
async fn open_http(url: &str, headers: &BTreeMap<String, String>) -> Result<Live, String> {
    otto_netguard::check_url(url).await?;
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("bad url: {e}"))?;
    let host = parsed.host_str().ok_or("url has no host")?.to_string();
    let port = parsed.port_or_known_default().ok_or("url has no port")?;
    let addrs = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("dns: {e}"))?;
    let addr = addrs
        .into_iter()
        .find(|a| !otto_netguard::is_blocked_ip(a.ip()))
        .ok_or("host resolves only to blocked addresses")?;
    let client = reqwest::Client::builder()
        .timeout(OP_TIMEOUT)
        .redirect(otto_netguard::redirect_policy())
        .resolve(&host, addr)
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let resp = http_send(&client, url, headers, init_request(), None)
        .await
        .map_err(|e| format!("initialize: {e}"))?;
    let session_id = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let _ = parse_http_message(resp).await?; // ensure initialize succeeded
                                             // notifications/initialized (best-effort).
    let _ = http_send(
        &client,
        url,
        headers,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        session_id.clone(),
    )
    .await;
    Ok(Live::Http {
        client,
        session_id,
        next_id: 2,
    })
}

fn http_send(
    client: &reqwest::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
    body: Value,
    session: Option<String>,
) -> impl std::future::Future<Output = reqwest::Result<reqwest::Response>> {
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
}

/// The request envelope with `id` right after `jsonrpc` (before `params`), so
/// the id is the first one on the wire whatever the map ordering.
fn with_id(request: &Value, id: i64) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("jsonrpc".into(), json!("2.0"));
    m.insert("id".into(), json!(id));
    if let Some(method) = request.get("method") {
        m.insert("method".into(), method.clone());
    }
    if let Some(params) = request.get("params") {
        m.insert("params".into(), params.clone());
    }
    Value::Object(m)
}

/// One request on a live session, with a fresh JSON-RPC id.
async fn run_on(live: &mut Live, transport: &Transport, request: Value) -> Result<Value, OpError> {
    match live {
        Live::Stdio { io, next_id } => {
            let StdioIo {
                child,
                stdin,
                reader,
            } = io.as_mut();
            // A child that already exited while parked never saw this request.
            if let Ok(Some(_)) = child.try_wait() {
                return Err(OpError::NotDelivered("server process exited".into()));
            }
            let id = *next_id;
            *next_id += 1;
            let request = with_id(&request, id);
            write_line(stdin, &request)
                .await
                .map_err(OpError::NotDelivered)?;
            read_until_id(reader, id).await.map_err(OpError::Failed)
        }
        Live::Http {
            client,
            session_id,
            next_id,
        } => {
            let Transport::Http { url, headers } = transport else {
                return Err(OpError::Failed("not an http transport".into()));
            };
            let id = *next_id;
            *next_id += 1;
            let request = with_id(&request, id);
            let resp = http_send(client, url, headers, request, session_id.clone())
                .await
                .map_err(|e| {
                    // A connect failure never reached the server.
                    if e.is_connect() {
                        OpError::NotDelivered(format!("op: {e}"))
                    } else {
                        OpError::Failed(format!("op: {e}"))
                    }
                })?;
            // Streamable HTTP: 404 on a request carrying a session id means the
            // server dropped the session and did NOT process the request.
            if resp.status() == reqwest::StatusCode::NOT_FOUND && session_id.is_some() {
                return Err(OpError::NotDelivered("mcp session expired".into()));
            }
            let msg = parse_http_message(resp).await.map_err(OpError::Failed)?;
            msg.get("result").cloned().ok_or_else(|| {
                OpError::Failed(
                    msg.get("error")
                        .map(|e| format!("server error: {e}"))
                        .unwrap_or_else(|| "no result in response".into()),
                )
            })
        }
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
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("read: {e}"))?;
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
    // Replies echo the request id (a pooled session numbers its requests).
    // `$SPAWNS` (optional) gets one line per server start; `$EXIT_AFTER_CALL`
    // makes the server exit after answering one tools/call; `$CALL_SLEEP`
    // (seconds) delays each tools/call answer.
    fn echo_server_script() -> String {
        r#"
[ -n "$SPAWNS" ] && echo x >> "$SPAWNS"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | grep -o '"id":[0-9]*' | head -n 1 | cut -d: -f2)
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mock","version":"1"}}}\n' ;;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"echo","inputSchema":{"type":"object"}}]}}\n' "$id" ;;
    *'"tools/call"'*) [ -n "$CALL_SLEEP" ] && sleep "$CALL_SLEEP"; printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"ok"}],"isError":false}}\n' "$id"; [ -n "$EXIT_AFTER_CALL" ] && exit 0 ;;
    *'"notifications/initialized"'*) : ;;
  esac
done
"#
        .to_string()
    }

    fn stdio_client_with(extra: &[(&str, String)]) -> McpClient {
        let mut env = BTreeMap::new();
        env.insert("LC_ALL".into(), "C".into());
        env.insert(
            "PATH".into(),
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
        );
        for (k, v) in extra {
            env.insert((*k).to_string(), v.clone());
        }
        McpClient::new(Transport::Stdio {
            command: "sh".into(),
            args: vec!["-c".into(), echo_server_script()],
            env,
        })
    }

    fn stdio_client() -> McpClient {
        stdio_client_with(&[])
    }

    fn spawns(path: &std::path::Path) -> usize {
        std::fs::read_to_string(path)
            .map(|s| s.lines().count())
            .unwrap_or(0)
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
        let r = stdio_client()
            .call_tool("echo", &json!({"x":1}))
            .await
            .unwrap();
        assert!(!r.is_error);
        assert_eq!(r.content["content"][0]["text"], json!("ok"));
    }

    // SE-14: one spawn + initialize serves every op on a client.
    #[tokio::test]
    async fn stdio_session_is_reused_across_ops() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("spawns");
        let c = stdio_client_with(&[("SPAWNS", log.display().to_string())]);
        c.list_tools().await.unwrap();
        for _ in 0..5 {
            let r = c.call_tool("echo", &json!({})).await.unwrap();
            assert_eq!(r.content["content"][0]["text"], json!("ok"));
        }
        assert_eq!(spawns(&log), 1, "six ops, one server process");
        assert!(c.has_live_session());
    }

    // A parked child that died while idle is replaced transparently: the
    // request never reached it, so the fresh retry is safe.
    #[tokio::test]
    async fn a_dead_parked_session_is_replaced_once() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("spawns");
        let c = stdio_client_with(&[
            ("SPAWNS", log.display().to_string()),
            ("EXIT_AFTER_CALL", "1".into()),
        ]);
        c.call_tool("echo", &json!({})).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await; // let it exit
        let r = c.call_tool("echo", &json!({})).await.unwrap();
        assert!(!r.is_error);
        assert_eq!(spawns(&log), 2);
    }

    // Concurrent ops all complete.
    #[tokio::test]
    async fn concurrent_ops_do_not_serialize() {
        let c = stdio_client();
        let args = json!({});
        let (a, b) = tokio::join!(c.call_tool("echo", &args), c.call_tool("echo", &args));
        assert!(a.is_ok() && b.is_ok());
    }

    // R3: a short op in flight is waited for (≤ BUSY_WAIT) instead of paying a
    // second server spawn — two concurrent calls, ONE process.
    #[tokio::test]
    async fn a_concurrent_call_reuses_the_busy_session_instead_of_spawning() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("spawns");
        let c = stdio_client_with(&[("SPAWNS", log.display().to_string())]);
        let args = json!({});
        let (a, b) = tokio::join!(c.call_tool("echo", &args), c.call_tool("echo", &args));
        assert!(a.is_ok() && b.is_ok());
        assert_eq!(
            spawns(&log),
            1,
            "the second call waited for the first session"
        );
    }

    // R3: ops slower than BUSY_WAIT open a SECOND parked session (not a
    // one-shot per call): later concurrent pairs reuse both — 2 spawns for
    // 3 rounds of 2 parallel calls (one-shots would be 1 + 3).
    #[tokio::test]
    async fn slow_parallel_calls_keep_a_pool_of_two_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("spawns");
        let c = stdio_client_with(&[
            ("SPAWNS", log.display().to_string()),
            ("CALL_SLEEP", "0.6".into()),
        ]);
        let args = json!({});
        for _ in 0..3 {
            let (a, b) = tokio::join!(c.call_tool("echo", &args), c.call_tool("echo", &args));
            assert!(a.is_ok() && b.is_ok());
        }
        assert_eq!(spawns(&log), 2, "two parked sessions serve every round");
    }
}
