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
//! spawn), then opens a second parked session in a free slot, and queues with a deadline when
//! all slots are busy. No operation opens more than these two sessions.
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

/// A governed call distinguishes admission denial from a transport failure.
#[derive(Debug)]
pub enum CheckedCallError {
    Denied(String),
    Transport(String),
}
impl From<String> for CheckedCallError {
    fn from(value: String) -> Self {
        Self::Transport(value)
    }
}
impl std::fmt::Display for CheckedCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied(s) | Self::Transport(s) => f.write_str(s),
        }
    }
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
        _permit: tokio::sync::OwnedSemaphorePermit,
    },
    Http {
        client: reqwest::Client,
        session_id: Option<String>,
        next_id: i64,
        _permit: tokio::sync::OwnedSemaphorePermit,
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

    /// The slot an op runs on, or `None` when the admission deadline expires.
    /// Order: an idle parked session → (nothing in flight) the first free slot
    /// → wait up to [`BUSY_WAIT`] for a busy session → a free slot (a second
    /// parked session) → wait for a slot up to the admission deadline.
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
        if let Some(slot) = waited.or(free) {
            return Some(slot);
        }
        // Admission is bounded by the parked slots. Queue with a deadline;
        // never spill a burst into additional server processes.
        tokio::time::timeout(OP_TIMEOUT, async {
            tokio::select! {
                slot = self.live[0].lock() => slot,
                slot = self.live[1].lock() => slot,
            }
        })
        .await
        .ok()
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
        self.call_tool_checked(name, args, || async { Ok(()) })
            .await
            .map_err(|e| e.to_string())
    }

    /// Reauthorize after admission and after each new transport's handshake.
    /// The hook also runs before the safe stale-session retry.
    pub async fn call_tool_checked<F, Fut>(
        &self,
        name: &str,
        args: &Value,
        authorize: F,
    ) -> Result<CallResult, CheckedCallError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<(), String>>,
    {
        let result = self
            .op_checked(
                json!({"jsonrpc":"2.0","method":"tools/call",
            "params":{"name":name,"arguments":args}}),
                authorize,
            )
            .await?;
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
        let mut slot = self
            .checkout()
            .await
            .ok_or_else(|| "MCP server busy; try again".to_string())?;
        // Probe startup within the same process budget, replacing this parked session.
        *slot = None;
        self.open().await.map(drop)
    }

    /// Run one post-initialize op, returning its `result`. Uses (and keeps) a
    /// parked session ([`Self::checkout`]); saturated callers queue with a
    /// deadline and never launch extra fallback transports.
    async fn op(&self, request: Value) -> Result<Value, String> {
        self.op_checked(request, || async { Ok(()) })
            .await
            .map_err(|e| e.to_string())
    }

    async fn op_checked<F, Fut>(
        &self,
        request: Value,
        mut authorize: F,
    ) -> Result<Value, CheckedCallError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<(), String>>,
    {
        let mut guard = self
            .checkout()
            .await
            .ok_or_else(|| "MCP server busy; try again".to_string())?;
        authorize().await.map_err(CheckedCallError::Denied)?;
        let reused = guard.is_some();
        // Own the transport outside the slot until success. Cancellation drops
        // it as well: unread responses must never leak into the next operation.
        let mut live = match guard.take() {
            Some(live) => live,
            None => self.open().await?,
        };
        if !reused {
            authorize().await.map_err(CheckedCallError::Denied)?;
        }
        let result = tokio::time::timeout(
            OP_TIMEOUT,
            run_on(&mut live, &self.transport, request.clone()),
        )
        .await;
        match result {
            Ok(Ok(value)) => {
                *guard = Some(live);
                Ok(value)
            }
            Ok(Err(OpError::NotDelivered(error))) if reused => {
                drop(live);
                authorize().await.map_err(CheckedCallError::Denied)?;
                let mut fresh = self.open().await?;
                authorize().await.map_err(CheckedCallError::Denied)?;
                let result =
                    tokio::time::timeout(OP_TIMEOUT, run_on(&mut fresh, &self.transport, request))
                        .await
                        .map_err(|_| timeout_msg())?
                        .map_err(|err| match err {
                            OpError::NotDelivered(e) | OpError::Failed(e) => {
                                format!("{e} (after stale session: {error})")
                            }
                        });
                if result.is_ok() {
                    *guard = Some(fresh);
                }
                result.map_err(CheckedCallError::Transport)
            }
            Ok(Err(OpError::NotDelivered(error) | OpError::Failed(error))) => {
                Err(CheckedCallError::Transport(error))
            }
            Err(_) => Err(CheckedCallError::Transport(timeout_msg())),
        }
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
                  "clientInfo":{"name":"otto-control-plane","version":env!("CARGO_PKG_VERSION")}}
    })
}

/// Bounds live transports across servers and config replacements, including
/// parked sessions. The permit is released on failure, cancellation and drop.
async fn transport_permit() -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    static LIMIT: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    let limit = LIMIT
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(64)))
        .clone();
    tokio::time::timeout(OP_TIMEOUT, limit.acquire_owned())
        .await
        .map_err(|_| "MCP transport budget busy; try again".to_string())?
        .map_err(|_| "MCP transport admission closed".to_string())
}

/// Parent variables a stdio MCP server inherits — the MCP SDK's
/// `getDefaultEnvironment` set (what a program needs to find binaries, a home,
/// a temp dir and a locale), plus the machine's network/runtime
/// CONFIGURATION a server cannot reach anything without: the proxy
/// variables (either case), the CA bundles a TLS-intercepting corporate
/// proxy needs, and `DOCKER_HOST` / `DOCKER_CONTEXT` (colima, OrbStack) for
/// `docker run …` servers. Everything else ottod holds (provider API keys,
/// AWS credentials, tokens under `cargo run` / a shell launch) stays out of a
/// third-party `npx` server; it gets only its configured env + secrets (the
/// server editor says so).
pub(crate) const STDIO_BASE_ENV: &[&str] = &[
    "HOME",
    "LOGNAME",
    "PATH",
    "SHELL",
    "TERM",
    "USER",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TMPDIR",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "DOCKER_HOST",
    "DOCKER_CONTEXT",
];

/// Prefixes of inherited parent variables: the XDG base directories
/// (`XDG_CONFIG_HOME`, `XDG_RUNTIME_DIR`, …) — paths, never secrets.
const STDIO_BASE_ENV_PREFIXES: &[&str] = &["XDG_"];

/// Whether a parent variable is passed to a stdio server. Pure — unit-tested.
pub(crate) fn is_stdio_base_env(key: &str) -> bool {
    STDIO_BASE_ENV.contains(&key) || STDIO_BASE_ENV_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// The allow-listed parent environment (see [`is_stdio_base_env`]).
pub(crate) fn stdio_base_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| is_stdio_base_env(k))
        .collect()
}

async fn open_stdio(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<Live, String> {
    let permit = transport_permit().await?;
    let mut child = Command::new(command)
        .args(args)
        .env_clear()
        .envs(stdio_base_env())
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
    let _ = read_until_id(&mut reader, Some(&mut stdin), 1).await?;
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
        _permit: permit,
    })
}

/// SSRF-validate the URL, build the guarded client and initialize.
async fn open_http(url: &str, headers: &BTreeMap<String, String>) -> Result<Live, String> {
    let permit = transport_permit().await?;
    // Up front for a clean error (and to vet an IP-literal host); the guarded
    // resolver then vets every name AT CONNECT TIME — the addresses it checks
    // are the ones dialled, so DNS cannot rebind between check and connect.
    otto_netguard::check_url(url).await?;
    let client = otto_netguard::guarded_client_builder()
        .timeout(OP_TIMEOUT)
        .redirect(same_origin_redirects())
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
    let _ = parse_http_message(resp, 1).await?; // ensure initialize succeeded
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
        _permit: permit,
    })
}

/// Redirects an MCP HTTP transport follows: SAME ORIGIN only (scheme, host and
/// port of the first request), bounded, and IP-literal hops re-vetted. A
/// cross-host 307/308 would re-send the JSON-RPC body (tool arguments) and
/// every configured custom header — reqwest strips only `Authorization` /
/// `Cookie`, not an `X-API-Key` read from the Keychain — to wherever the
/// server (or anything on its path) points. No DNS on the callback thread.
fn same_origin_redirects() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        let origin = attempt.previous().first();
        if origin.is_some_and(|o| same_origin(o, attempt.url()))
            && otto_netguard::check_url_literal(attempt.url())
        {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

/// Redirect hops [`same_origin_redirects`] follows.
const MAX_REDIRECTS: usize = 5;

/// Same scheme, host and (effective) port. Pure — unit-tested.
fn same_origin(a: &reqwest::Url, b: &reqwest::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
        && a.port_or_known_default() == b.port_or_known_default()
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
        Live::Stdio { io, next_id, .. } => {
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
            read_until_id(reader, Some(stdin), id)
                .await
                .map_err(OpError::Failed)
        }
        Live::Http {
            client,
            session_id,
            next_id,
            ..
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
            // Server→client requests on the stream are answered with a POST
            // of their own (spawned: the stream is still being read).
            let answer = |reply: Value| {
                let (client, url, headers, sid) = (
                    client.clone(),
                    url.clone(),
                    headers.clone(),
                    session_id.clone(),
                );
                tokio::spawn(async move {
                    let _ = http_send(&client, &url, &headers, reply, sid).await;
                });
            };
            let msg = parse_http_message_answering(resp, id, &answer)
                .await
                .map_err(OpError::Failed)?;
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

/// Read the JSON-RPC response to request `want_id` from an HTTP response,
/// honoring content negotiation (a JSON body, or an SSE `text/event-stream`).
/// Body is size-capped.
async fn parse_http_message(resp: reqwest::Response, want_id: i64) -> Result<Value, String> {
    parse_http_message_answering(resp, want_id, &|_| {}).await
}

/// [`parse_http_message`], answering every server→client request seen on an
/// SSE stream through `answer` (a POST back to the server) AS IT ARRIVES —
/// a server that blocks on the answer before sending the result no longer
/// holds the stream open to the timeout. The SSE body is parsed
/// incrementally and the call returns as soon as its response is read.
async fn parse_http_message_answering(
    mut resp: reqwest::Response,
    want_id: i64,
    answer: &(dyn Fn(Value) + Sync),
) -> Result<Value, String> {
    let status = resp.status();
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if status.is_success() && ct.contains("text/event-stream") {
        let mut parser = SseParser::default();
        let mut fallback = None;
        let mut total = 0usize;
        loop {
            let chunk = resp.chunk().await.map_err(|e| format!("read body: {e}"))?;
            let mut msgs = Vec::new();
            match &chunk {
                Some(c) => {
                    total += c.len();
                    if total > MAX_BODY_BYTES {
                        return Err("response exceeded size cap".into());
                    }
                    parser.feed(c, &mut msgs);
                }
                None => parser.finish(&mut msgs),
            }
            for m in msgs {
                if let Some(reply) = server_request_answer(&m) {
                    answer(reply);
                    continue;
                }
                if !is_response(&m) {
                    continue;
                }
                if m.get("id").and_then(Value::as_i64) == Some(want_id) {
                    return Ok(m);
                }
                fallback.get_or_insert(m);
            }
            if chunk.is_none() {
                break;
            }
        }
        if !parser.saw_data {
            return Err("empty SSE stream".into());
        }
        return fallback.ok_or_else(|| "no response for this request in SSE stream".into());
    }
    if resp
        .content_length()
        .is_some_and(|len| len > MAX_BODY_BYTES as u64)
    {
        return Err("response exceeded size cap".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("read body: {e}"))? {
        if chunk.len() > MAX_BODY_BYTES - bytes.len() {
            return Err("response exceeded size cap".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let text = String::from_utf8_lossy(&bytes);
    if !status.is_success() {
        let snippet: String = text.chars().take(300).collect();
        return Err(format!("http {status}: {snippet}"));
    }
    if ct.contains("text/event-stream") {
        pick_sse_response(&text, want_id)
    } else {
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse json: {e}"))?;
        // A JSON-RPC batch body: pick our response out of it.
        match v {
            Value::Array(msgs) => pick_response(msgs, want_id)
                .ok_or_else(|| "no response for this request in batch".to_string()),
            v => Ok(v),
        }
    }
}

/// Split an SSE body into events (blank-line separated; an event's `data:`
/// lines join with `\n` per the SSE spec), parse each event as one JSON-RPC
/// message, and return the response to `want_id`. A Streamable-HTTP server may
/// emit notifications (progress, logging) and server→client requests on the
/// same stream BEFORE the result — those are skipped, not concatenated into
/// the result (which used to make the whole body unparseable).
fn pick_sse_response(text: &str, want_id: i64) -> Result<Value, String> {
    let mut p = SseParser::default();
    let mut msgs = Vec::new();
    p.feed(text.as_bytes(), &mut msgs);
    p.finish(&mut msgs);
    if !p.saw_data {
        return Err("empty SSE stream".into());
    }
    pick_response(msgs, want_id).ok_or_else(|| "no response for this request in SSE stream".into())
}

/// The answer to a server→client REQUEST (a message with `method` AND `id`):
/// `ping` → `{}`; anything else (`sampling/createMessage`,
/// `elicitation/create`, `roots/list`, …) → -32601, since Otto offers none of
/// those capabilities. `None` for a notification or a response. A server
/// that blocks on the answer must get one — else the call hangs to the
/// timeout and the session is dropped.
fn server_request_answer(msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str)?;
    let id = msg.get("id")?;
    Some(if method == "ping" {
        json!({"jsonrpc":"2.0","id":id,"result":{}})
    } else {
        json!({"jsonrpc":"2.0","id":id,
            "error":{"code":-32601,"message":"method not found"}})
    })
}

/// Incremental `text/event-stream` → JSON-RPC message parser: events are
/// blank-line separated, an event's `data:` lines join with `\n` (SSE spec),
/// and each event is one JSON-RPC message. Fed chunk by chunk, so a message
/// is acted on as soon as its event completes — not when the body ends.
#[derive(Default)]
struct SseParser {
    /// Bytes of an incomplete line (a chunk may split a line or a UTF-8 char).
    pending: Vec<u8>,
    data: Option<String>,
    saw_data: bool,
}

impl SseParser {
    fn feed(&mut self, chunk: &[u8], out: &mut Vec<Value>) {
        self.pending.extend_from_slice(chunk);
        while let Some(i) = self.pending.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.pending.drain(..=i).collect();
            let line = String::from_utf8_lossy(&raw[..raw.len() - 1]).into_owned();
            self.line(line.strip_suffix('\r').unwrap_or(&line), out);
        }
    }

    /// End of stream: the last line and event need no trailing terminator.
    fn finish(&mut self, out: &mut Vec<Value>) {
        if !self.pending.is_empty() {
            let raw = std::mem::take(&mut self.pending);
            let line = String::from_utf8_lossy(&raw).into_owned();
            self.line(line.strip_suffix('\r').unwrap_or(&line), out);
        }
        self.flush(out);
    }

    fn line(&mut self, line: &str, out: &mut Vec<Value>) {
        if line.is_empty() {
            self.flush(out);
            return;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            self.saw_data = true;
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            match &mut self.data {
                Some(d) => {
                    d.push('\n');
                    d.push_str(rest);
                }
                None => self.data = Some(rest.to_string()),
            }
        }
        // `event:` / `id:` / `retry:` / `:comment` lines carry no payload.
    }

    fn flush(&mut self, out: &mut Vec<Value>) {
        if let Some(d) = self.data.take() {
            if let Ok(v) = serde_json::from_str::<Value>(&d) {
                out.push(v);
            }
        }
    }
}

/// Is `msg` a JSON-RPC *response* (not a notification / server request)?
fn is_response(msg: &Value) -> bool {
    msg.get("method").is_none() && (msg.get("result").is_some() || msg.get("error").is_some())
}

/// The response whose id is `want_id`; failing that, the first response at
/// all (a server that echoes a different id type, e.g. a string).
fn pick_response(msgs: Vec<Value>, want_id: i64) -> Option<Value> {
    let mut fallback = None;
    for m in msgs {
        if !is_response(&m) {
            continue;
        }
        if m.get("id").and_then(Value::as_i64) == Some(want_id) {
            return Some(m);
        }
        if fallback.is_none() {
            fallback = Some(m);
        }
    }
    fallback
}

async fn write_line<W: AsyncWriteExt + Unpin>(w: &mut W, v: &Value) -> Result<(), String> {
    let mut buf = serde_json::to_vec(v).map_err(|e| format!("encode: {e}"))?;
    buf.push(b'\n');
    w.write_all(&buf).await.map_err(|e| format!("write: {e}"))?;
    w.flush().await.map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

/// Read newline-delimited JSON-RPC lines until the *response* with
/// `id == want_id` arrives, return its `result` (or surface its `error`).
/// Notifications and responses to other ids are skipped. A server→client
/// request (it has a `method`) is never mistaken for our response even when
/// its id collides with ours: a `ping` is answered with `{}`, anything else
/// with "method not found", on `reply` when given (best-effort).
async fn read_until_id<R: AsyncBufReadExt + Unpin, W: AsyncWriteExt + Unpin>(
    reader: &mut R,
    mut reply: Option<&mut W>,
    want_id: i64,
) -> Result<Value, String> {
    let mut line = Vec::new();
    let mut total = 0usize;
    loop {
        line.clear();
        loop {
            let available = reader.fill_buf().await.map_err(|e| format!("read: {e}"))?;
            if available.is_empty() {
                if line.is_empty() {
                    return Err("server closed before responding".into());
                }
                break;
            }
            let n = available
                .iter()
                .position(|b| *b == b'\n')
                .map_or(available.len(), |i| i + 1);
            if n > MAX_BODY_BYTES - total {
                return Err("response exceeded size cap".into());
            }
            let complete = available[n - 1] == b'\n';
            line.extend_from_slice(&available[..n]);
            total += n;
            reader.consume(n);
            if complete {
                break;
            }
        }
        let text = String::from_utf8_lossy(&line);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue, // tolerate stray non-JSON noise on stdout
        };
        if msg.get("method").is_some() {
            // Server→client request (has an id) or notification (no id).
            if let (Some(answer), Some(w)) = (server_request_answer(&msg), reply.as_deref_mut()) {
                let _ = write_line(w, &answer).await;
            }
            continue;
        }
        let id_matches = msg.get("id").and_then(Value::as_i64) == Some(want_id);
        if !id_matches || !is_response(&msg) {
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

    #[tokio::test]
    async fn newline_free_response_stops_at_cap() {
        use tokio::io::AsyncWriteExt;
        let (mut writer, reader) = tokio::io::duplex(8192);
        let producer = tokio::spawn(async move {
            loop {
                if writer.write_all(&[b'x'; 8192]).await.is_err() {
                    break;
                }
            }
        });
        let mut reader = BufReader::new(reader);
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            read_until_id(&mut reader, None::<&mut tokio::io::Sink>, 1),
        )
        .await;
        drop(reader);
        producer.await.unwrap();
        assert!(result.unwrap().unwrap_err().contains("size cap"));
    }

    #[tokio::test]
    async fn notifications_share_the_response_budget() {
        let notification = b"{\"method\":\"progress\"}\n";
        let bytes = notification.repeat(MAX_BODY_BYTES / notification.len() + 1);
        let mut reader = BufReader::new(bytes.as_slice());
        assert!(read_until_id(&mut reader, None::<&mut tokio::io::Sink>, 1)
            .await
            .unwrap_err()
            .contains("size cap"));
    }

    #[tokio::test]
    async fn chunked_http_and_sse_stop_at_cap_before_the_stream_ends() {
        use tokio::io::AsyncWriteExt;
        for content_type in ["application/json", "text/event-stream"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let producer = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                socket.write_all(format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\n\r\n"
                ).as_bytes()).await.unwrap();
                // Intentionally never send the terminating HTTP chunk. The
                // reader must reject over-budget bytes before waiting for EOF.
                let chunk = [b'x'; 8192];
                loop {
                    if socket.write_all(b"2000\r\n").await.is_err()
                        || socket.write_all(&chunk).await.is_err()
                        || socket.write_all(b"\r\n").await.is_err()
                    {
                        break;
                    }
                }
            });
            let result = tokio::time::timeout(Duration::from_secs(3), async {
                let response = reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .unwrap()
                    .get(format!("http://{address}/stream"))
                    .send()
                    .await
                    .unwrap();
                parse_http_message(response, 1).await
            })
            .await;
            producer.abort();
            let _ = producer.await;
            assert!(result
                .expect("size cap must precede EOF")
                .unwrap_err()
                .contains("size cap"));
        }
    }

    #[test]
    fn sse_notification_before_result_returns_the_result() {
        // Streamable HTTP: progress/log notifications and a server request
        // may precede the response on the same stream.
        let body = "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{\"progress\":1}}\r\n\r\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n\n\
                    : keep-alive\n\n\
                    event: message\n\
                    data: {\"jsonrpc\":\"2.0\",\n\
                    data: \"id\":7,\"result\":{\"ok\":true}}\n\n";
        let msg = pick_sse_response(body, 7).unwrap();
        assert_eq!(msg["result"]["ok"], json!(true));
        // A stream with only a notification has no response.
        let only = "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\"}\n\n";
        assert!(pick_sse_response(only, 7).is_err());
        assert!(pick_sse_response("event: x\n\n", 7)
            .unwrap_err()
            .contains("empty"));
    }

    #[test]
    fn pick_response_prefers_matching_id_then_any_response() {
        let msgs = vec![
            json!({"jsonrpc":"2.0","id":1,"result":{"n":1}}),
            json!({"jsonrpc":"2.0","id":2,"error":{"code":1}}),
        ];
        assert_eq!(pick_response(msgs.clone(), 2).unwrap()["error"]["code"], 1);
        assert_eq!(pick_response(msgs, 9).unwrap()["result"]["n"], 1);
    }

    #[tokio::test]
    async fn stdio_server_request_with_colliding_id_is_answered_not_returned() {
        // A server `ping` reusing our id must not be taken as our response.
        let lines = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"real\":true}}\n",
        );
        let mut reader = BufReader::new(lines.as_bytes());
        let mut out: Vec<u8> = Vec::new();
        let v = read_until_id(&mut reader, Some(&mut out), 3).await.unwrap();
        assert_eq!(v["real"], json!(true));
        let answered: Value = serde_json::from_slice(out.trim_ascii()).unwrap();
        assert_eq!(answered["id"], json!(3));
        assert_eq!(answered["result"], json!({}), "ping answered with {{}}");
    }

    /// S5-12: a stdio server inherits only the allow-listed base — never,
    /// say, the cargo / provider variables of the daemon's environment.
    #[test]
    fn stdio_servers_inherit_only_the_base_environment() {
        let base = stdio_base_env();
        assert!(base.iter().all(|(k, _)| is_stdio_base_env(k)));
        assert!(base.iter().any(|(k, _)| k == "PATH"), "PATH is passed");
        // cargo sets this for the test process; it must not leak through.
        assert!(std::env::var("CARGO_MANIFEST_DIR").is_ok());
        assert!(!base.iter().any(|(k, _)| k == "CARGO_MANIFEST_DIR"));
    }

    /// S5-307: proxy, CA-bundle, Docker and XDG configuration still reach a
    /// stdio server (a corporate proxy / colima user upgrading must not lose
    /// them); credentials and arbitrary variables do not.
    #[test]
    fn network_configuration_reaches_stdio_servers_but_credentials_do_not() {
        for k in [
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "NO_PROXY",
            "no_proxy",
            "NODE_EXTRA_CA_CERTS",
            "SSL_CERT_FILE",
            "REQUESTS_CA_BUNDLE",
            "DOCKER_HOST",
            "XDG_CONFIG_HOME",
            "XDG_RUNTIME_DIR",
            "PATH",
        ] {
            assert!(is_stdio_base_env(k), "{k}");
        }
        for k in [
            "AWS_SECRET_ACCESS_KEY",
            "ANTHROPIC_API_KEY",
            "GITHUB_TOKEN",
            "OTTO_TOKEN",
            "CARGO_MANIFEST_DIR",
            "XDG",
            "DOCKER_CONFIG",
        ] {
            assert!(!is_stdio_base_env(k), "{k}");
        }
    }

    /// S5-09: a server→client request on the SSE stream is answered (-32601,
    /// ping → {}) WHILE the stream is open, and the result that follows it
    /// is returned without waiting for the body to end.
    #[tokio::test]
    async fn a_server_request_on_the_stream_is_answered_while_it_is_open() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel::<String>();
        let server = tokio::spawn(async move {
            // 1. The call: stream a server request, then hold the stream.
            let (mut call, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = call.read(&mut buf).await;
            call.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
            )
            .await
            .unwrap();
            let ev =
                "data: {\"jsonrpc\":\"2.0\",\"id\":99,\"method\":\"sampling/createMessage\"}\n\n";
            call.write_all(format!("{:x}\r\n{ev}\r\n", ev.len()).as_bytes())
                .await
                .unwrap();
            // 2. Our answer arrives on a second connection.
            let (mut ans, _) = listener.accept().await.unwrap();
            let mut abuf = vec![0u8; 8192];
            let n = ans.read(&mut abuf).await.unwrap();
            let _ = ans
                .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
                .await;
            let _ = seen_tx.send(String::from_utf8_lossy(&abuf[..n]).into_owned());
            // 3. Only now the result (the stream stays open after it).
            let ev = "data: {\"jsonrpc\":\"2.0\",\"id\":5,\"result\":{\"ok\":true}}\n\n";
            call.write_all(format!("{:x}\r\n{ev}\r\n", ev.len()).as_bytes())
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://{address}/mcp");
        let resp = http_send(&client, &url, &BTreeMap::new(), init_request(), None)
            .await
            .unwrap();
        let (c2, u2) = (client.clone(), url.clone());
        let answer = move |reply: Value| {
            let (c, u) = (c2.clone(), u2.clone());
            tokio::spawn(async move {
                let _ = http_send(&c, &u, &BTreeMap::new(), reply, None).await;
            });
        };
        let msg = tokio::time::timeout(
            Duration::from_secs(5),
            parse_http_message_answering(resp, 5, &answer),
        )
        .await
        .expect("returned on the result, not the stream's end")
        .unwrap();
        assert_eq!(msg["result"]["ok"], json!(true));
        let posted = seen_rx.await.unwrap();
        assert!(posted.contains("\"id\":99"), "{posted}");
        assert!(posted.contains("-32601"), "{posted}");
        server.abort();
    }

    #[test]
    fn redirects_stay_on_the_origin() {
        let u = |s: &str| reqwest::Url::parse(s).unwrap();
        assert!(same_origin(
            &u("https://mcp.example/a"),
            &u("https://MCP.example:443/b")
        ));
        assert!(!same_origin(
            &u("https://mcp.example/a"),
            &u("https://attacker.example/a")
        ));
        assert!(!same_origin(
            &u("https://mcp.example/a"),
            &u("http://mcp.example/a")
        ));
        assert!(!same_origin(
            &u("https://mcp.example/a"),
            &u("https://mcp.example:8443/a")
        ));
    }

    /// S5-08: a cross-host 307 is NOT followed, so the secret header and the
    /// JSON-RPC body never reach the redirect target.
    #[tokio::test]
    async fn a_cross_host_redirect_does_not_forward_the_secret_header() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target.local_addr().unwrap();
        let hit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hit2 = hit.clone();
        tokio::spawn(async move {
            if let Ok((mut s, _)) = target.accept().await {
                hit2.store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = s
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        });
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = origin.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf).await;
            // `localhost` ≠ `127.0.0.1`: a different host (and port).
            let head = format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://localhost:{}/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                target_addr.port()
            );
            let _ = s.write_all(head.as_bytes()).await;
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(same_origin_redirects())
            .build()
            .unwrap();
        let mut headers = BTreeMap::new();
        headers.insert("X-API-Key".to_string(), "keychain-secret".to_string());
        let resp = http_send(
            &client,
            &format!("http://{origin_addr}/mcp"),
            &headers,
            init_request(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(resp.status().as_u16(), 307, "the hop was not followed");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !hit.load(std::sync::atomic::Ordering::SeqCst),
            "target never contacted"
        );
    }

    #[tokio::test]
    async fn http_sse_response_after_notification_end_to_end() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let body = "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n\
                        data: {\"jsonrpc\":\"2.0\",\"id\":5,\"result\":{\"tools\":[]}}\n\n";
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
        });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/mcp"))
            .send()
            .await
            .unwrap();
        let msg = parse_http_message(response, 5).await.unwrap();
        assert_eq!(msg["result"]["tools"], json!([]));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_burst_keeps_two_server_processes() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("spawns");
        let client = std::sync::Arc::new(stdio_client_with(&[
            ("SPAWNS", log.display().to_string()),
            ("CALL_SLEEP", "0.3".into()),
        ]));
        let mut calls = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let client = client.clone();
            calls.spawn(async move { client.call_tool("echo", &json!({})).await });
        }
        while let Some(result) = calls.join_next().await {
            assert!(result.unwrap().is_ok());
        }
        assert!(spawns(&log) <= SESSION_SLOTS);
    }

    #[tokio::test]
    async fn queued_call_rechecks_authorization_after_admission() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("spawns");
        let client = stdio_client_with(&[("SPAWNS", log.display().to_string())]);
        let first = client.checkout().await.unwrap();
        let second = client.checkout().await.unwrap();
        let allowed = std::sync::atomic::AtomicBool::new(true);
        let args = json!({});
        let call = client.call_tool_checked("echo", &args, || async {
            if allowed.load(std::sync::atomic::Ordering::SeqCst) {
                Ok(())
            } else {
                Err("revoked while queued".into())
            }
        });
        tokio::pin!(call);
        assert!(tokio::time::timeout(Duration::from_millis(30), &mut call)
            .await
            .is_err());
        allowed.store(false, std::sync::atomic::Ordering::SeqCst);
        drop(first);
        drop(second);
        assert!(matches!(call.await, Err(CheckedCallError::Denied(_))));
        assert_eq!(spawns(&log), 0, "revoked call must not open a transport");
    }

    #[tokio::test]
    async fn fresh_transport_rechecks_after_initialize_before_tool_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let calls = dir.path().join("calls");
        let log = dir.path().join("spawns");
        let client = stdio_client_with(&[
            ("SPAWNS", log.display().to_string()),
            ("CALLS", calls.display().to_string()),
        ]);
        let mut checks = 0;
        let result = client
            .call_tool_checked("echo", &json!({}), || {
                checks += 1;
                std::future::ready(if checks == 1 {
                    Ok(())
                } else {
                    Err("revoked during handshake".into())
                })
            })
            .await;
        assert!(matches!(result, Err(CheckedCallError::Denied(_))));
        assert_eq!(spawns(&log), 1, "initialize really ran");
        assert_eq!(spawns(&calls), 0, "tools/call was never delivered");
    }

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
    *'"tools/call"'*) [ -n "$CALLS" ] && echo x >> "$CALLS"; [ -n "$CALL_SLEEP" ] && sleep "$CALL_SLEEP"; printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"ok"}],"isError":false}}\n' "$id"; [ -n "$EXIT_AFTER_CALL" ] && exit 0 ;;
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
