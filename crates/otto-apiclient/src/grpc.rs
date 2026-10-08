//! gRPC / protobuf engine behind the API client's endpoints (the thin axum
//! handlers — auth + the workspace allow-local lookup — live in
//! `otto-server`'s `routes::grpc`):
//!   POST /workspaces/{wid}/api-client/grpc/describe  — parse a `.proto`, list
//!        services/methods + a JSON request skeleton (viewable descriptors).
//!   POST /workspaces/{wid}/api-client/grpc/invoke    — dynamically invoke a
//!        unary method (JSON in → JSON out) through a real gRPC connection.
//!   POST /workspaces/{wid}/api-client/grpc/reflect   — list services/methods
//!        via the server's reflection API.
//!
//! Parsing uses `protox` (no `protoc` needed); dynamic messages use
//! `prost-reflect`; transport uses `tonic` with a descriptor-driven codec.

use std::str::FromStr;
use std::time::{Duration, Instant};

use http::uri::PathAndQuery;
use prost::Message as _;
use prost_reflect::{DescriptorPool, DynamicMessage, Kind, MessageDescriptor, MethodDescriptor};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tonic::Status;

use otto_core::api::{ApiResponse, TraceStep};
use otto_core::Error;

fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}
fn upstream(msg: impl Into<String>) -> Error {
    Error::Upstream(msg.into())
}

// ── caches (perf F6) ────────────────────────────────────────────────────────
//
// Without these every invoke recompiled its .proto (protox, on a tokio
// worker), or — with no .proto — recompiled the reflection proto, dialled the
// server for reflection, then dialled it AGAIN for the call: two TCP+TLS
// handshakes plus 2+ reflection RPCs per click.

/// A small process-wide TTL cache, bounded by entry count (oldest out first).
struct TtlCache<V> {
    map: std::collections::HashMap<String, (V, Instant)>,
    ttl: Duration,
    cap: usize,
}

impl<V: Clone> TtlCache<V> {
    fn new(ttl: Duration, cap: usize) -> Self {
        Self {
            map: std::collections::HashMap::new(),
            ttl,
            cap,
        }
    }

    /// A live entry, refreshing its timestamp (idle TTL).
    fn get(&mut self, key: &str) -> Option<V> {
        let ttl = self.ttl;
        self.map.retain(|_, (_, at)| at.elapsed() < ttl);
        let (value, at) = self.map.get_mut(key)?;
        *at = Instant::now();
        Some(value.clone())
    }

    fn put(&mut self, key: String, value: V) {
        if !self.map.contains_key(&key) && self.map.len() >= self.cap {
            if let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(k, _)| k.clone())
            {
                self.map.remove(&oldest);
            }
        }
        self.map.insert(key, (value, Instant::now()));
    }

    fn remove(&mut self, key: &str) {
        self.map.remove(key);
    }
}

type Shared<V> = std::sync::Mutex<TtlCache<V>>;

fn lock<V>(m: &Shared<V>) -> std::sync::MutexGuard<'_, TtlCache<V>> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Compiled descriptor pools: keyed by sha256 of the .proto source (an
/// uploaded proto) or of url + allow-local + metadata (a reflected one).
fn pool_cache() -> &'static Shared<DescriptorPool> {
    static C: std::sync::OnceLock<Shared<DescriptorPool>> = std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(TtlCache::new(Duration::from_secs(5 * 60), 32)))
}

/// Connected channels by `(url, allow_local)`; tonic `Channel`s multiplex and
/// clone cheaply. 2 min idle TTL.
fn channel_cache() -> &'static Shared<Channel> {
    static C: std::sync::OnceLock<Shared<Channel>> = std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(TtlCache::new(Duration::from_secs(2 * 60), 32)))
}

fn sha_key(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p.as_bytes());
    }
    hex::encode(h.finalize())
}

/// How many times a .proto was actually compiled (cache misses) — the
/// measurability hook for the cache tests.
static PROTO_COMPILES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How many channels were actually dialled (channel-cache misses) — the
/// measurability hook for the channel-reuse test (perf N5).
static CHANNEL_CONNECTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// [`pool_from_proto`] through the pool cache, compiling on the blocking pool
/// (protox + a tempdir write are synchronous file I/O and CPU).
async fn pool_from_proto_cached(proto: &str) -> Result<DescriptorPool, Error> {
    let key = format!("proto:{}", sha_key(&[proto]));
    if let Some(pool) = lock(pool_cache()).get(&key) {
        return Ok(pool);
    }
    let src = proto.to_string();
    let pool = tokio::task::spawn_blocking(move || pool_from_proto(&src))
        .await
        .map_err(|e| upstream(format!("proto compile failed: {e}")))??;
    lock(pool_cache()).put(key, pool.clone());
    Ok(pool)
}

/// A channel to `url`, reused from [`channel_cache`] when one is live.
async fn cached_channel(url: &str, allow_local: bool) -> Result<Channel, Error> {
    let key = format!("{allow_local}\n{url}");
    if let Some(ch) = lock(channel_cache()).get(&key) {
        return Ok(ch);
    }
    let ch = connect_channel(url, allow_local).await?;
    lock(channel_cache()).put(key, ch.clone());
    Ok(ch)
}

/// Drop a cached channel after a transport failure so the next call redials.
fn forget_channel(url: &str, allow_local: bool) {
    lock(channel_cache()).remove(&format!("{allow_local}\n{url}"));
}

fn reflect_key(url: &str, headers: &[KV], allow_local: bool) -> String {
    let mut parts: Vec<&str> = vec![url, if allow_local { "1" } else { "0" }];
    for h in headers {
        parts.push(&h.key);
        parts.push(&h.value);
    }
    format!("reflect:{}", sha_key(&parts))
}

/// [`reflect_pool`] through the pool cache. `fresh` (the explicit "list
/// services" action) re-reflects and refreshes the entry.
async fn reflect_pool_cached(
    url: &str,
    headers: &[KV],
    allow_local: bool,
    fresh: bool,
) -> Result<DescriptorPool, Error> {
    let key = reflect_key(url, headers, allow_local);
    if !fresh {
        if let Some(pool) = lock(pool_cache()).get(&key) {
            return Ok(pool);
        }
    }
    let pool = reflect_pool(url, headers, allow_local).await?;
    lock(pool_cache()).put(key, pool.clone());
    Ok(pool)
}

// ── describe ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct GrpcDescribeReq {
    #[serde(default)]
    pub proto: String,
}

#[derive(Serialize)]
pub struct GrpcMethodInfo {
    name: String,
    /// Call path: `/package.Service/Method`.
    full: String,
    input_type: String,
    output_type: String,
    /// JSON skeleton for the request message.
    input_schema: String,
    client_streaming: bool,
    server_streaming: bool,
}

#[derive(Serialize)]
pub struct GrpcServiceInfo {
    name: String,
    methods: Vec<GrpcMethodInfo>,
}

#[derive(Serialize)]
pub struct GrpcDescribeResp {
    services: Vec<GrpcServiceInfo>,
}

/// Compile `.proto` source into a `prost-reflect` descriptor pool.
fn pool_from_proto(proto: &str) -> Result<DescriptorPool, Error> {
    if proto.trim().is_empty() {
        return Err(invalid("empty .proto"));
    }
    PROTO_COMPILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = tempfile::tempdir().map_err(|e| upstream(e.to_string()))?;
    let proto_path = dir.path().join("service.proto");
    std::fs::write(&proto_path, proto).map_err(|e| upstream(e.to_string()))?;
    let fds = protox::compile([&proto_path], [dir.path()])
        .map_err(|e| invalid(format!("proto parse error: {e}")))?;
    // Round-trip through bytes so prost-types version identity never matters.
    let mut buf = Vec::new();
    fds.encode(&mut buf)
        .map_err(|e| upstream(format!("encode descriptors: {e}")))?;
    DescriptorPool::decode(buf.as_slice())
        .map_err(|e| invalid(format!("descriptor build error: {e}")))
}

/// Extract the service/method list (with request skeletons) from a pool.
fn services_from_pool(pool: &DescriptorPool) -> Vec<GrpcServiceInfo> {
    let mut services = Vec::new();
    for service in pool.services() {
        if service.full_name().starts_with("grpc.reflection") {
            continue;
        }
        let mut methods = Vec::new();
        for method in service.methods() {
            methods.push(GrpcMethodInfo {
                name: method.name().to_string(),
                full: format!("/{}/{}", service.full_name(), method.name()),
                input_type: method.input().full_name().to_string(),
                output_type: method.output().full_name().to_string(),
                input_schema: json_skeleton(&method.input(), 0),
                client_streaming: method.is_client_streaming(),
                server_streaming: method.is_server_streaming(),
            });
        }
        services.push(GrpcServiceInfo {
            name: service.full_name().to_string(),
            methods,
        });
    }
    services
}

/// Compile the uploaded `.proto` (cached) and list its services/methods.
pub async fn describe(req: &GrpcDescribeReq) -> Result<GrpcDescribeResp, Error> {
    let pool = pool_from_proto_cached(&req.proto).await?;
    let services = services_from_pool(&pool);
    Ok(GrpcDescribeResp { services })
}

/// Build a JSON skeleton for a message (depth-limited to avoid recursion blowups).
fn json_skeleton(msg: &MessageDescriptor, depth: u8) -> String {
    serde_json::to_string_pretty(&skeleton_value(msg, depth)).unwrap_or_else(|_| "{}".to_string())
}

fn skeleton_value(msg: &MessageDescriptor, depth: u8) -> Value {
    let mut map = serde_json::Map::new();
    for field in msg.fields() {
        map.insert(
            field.json_name().to_string(),
            field_placeholder(&field, depth),
        );
    }
    Value::Object(map)
}

fn field_placeholder(field: &prost_reflect::FieldDescriptor, depth: u8) -> Value {
    if field.is_list() {
        return Value::Array(Vec::new());
    }
    if field.is_map() {
        return Value::Object(serde_json::Map::new());
    }
    match field.kind() {
        Kind::Double
        | Kind::Float
        | Kind::Int32
        | Kind::Int64
        | Kind::Uint32
        | Kind::Uint64
        | Kind::Sint32
        | Kind::Sint64
        | Kind::Fixed32
        | Kind::Fixed64
        | Kind::Sfixed32
        | Kind::Sfixed64 => json!(0),
        Kind::Bool => json!(false),
        Kind::String | Kind::Bytes => json!(""),
        Kind::Enum(_) => json!(0),
        Kind::Message(m) => {
            if depth >= 3 {
                Value::Object(serde_json::Map::new())
            } else {
                skeleton_value(&m, depth + 1)
            }
        }
    }
}

// ── invoke ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct KV {
    #[serde(default)]
    key: String,
    #[serde(default)]
    value: String,
}

#[derive(Deserialize)]
pub struct GrpcInvokeReq {
    url: String,
    #[serde(default)]
    proto: String,
    /// `/package.Service/Method`.
    method: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    headers: Vec<KV>,
}

/// Budget for one gRPC call (unary answer, or the whole server stream).
const GRPC_CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Server-stream caps: messages kept, and their serialized JSON size.
const GRPC_STREAM_MAX_MSGS: usize = 1000;
const GRPC_STREAM_MAX_BYTES: usize = 5 * 1024 * 1024;

fn deadline_status() -> Status {
    Status::deadline_exceeded(format!(
        "no response within {}s",
        GRPC_CALL_TIMEOUT.as_secs()
    ))
}

/// Bounded accumulator for server-streamed messages.
#[derive(Default)]
struct StreamCollector {
    msgs: Vec<Value>,
    bytes: usize,
    truncated: bool,
}

impl StreamCollector {
    /// Keep `msg` unless a cap is reached; `false` (and `truncated`) once the
    /// stream should stop being read.
    fn push(&mut self, msg: Value) -> bool {
        let size = msg.to_string().len();
        if self.msgs.len() >= GRPC_STREAM_MAX_MSGS || self.bytes + size > GRPC_STREAM_MAX_BYTES {
            self.truncated = true;
            return false;
        }
        self.bytes += size;
        self.msgs.push(msg);
        true
    }
}

/// A `tonic::Codec` that encodes/decodes `prost-reflect` dynamic messages
/// against a specific method's input/output descriptors.
#[derive(Clone)]
struct DynamicCodec {
    output: MessageDescriptor,
}

impl Codec for DynamicCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynEncoder;
    type Decoder = DynDecoder;
    fn encoder(&mut self) -> Self::Encoder {
        DynEncoder
    }
    fn decoder(&mut self) -> Self::Decoder {
        DynDecoder {
            output: self.output.clone(),
        }
    }
}

struct DynEncoder;
impl Encoder for DynEncoder {
    type Item = DynamicMessage;
    type Error = Status;
    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        item.encode(dst)
            .map_err(|e| Status::internal(e.to_string()))
    }
}

struct DynDecoder {
    output: MessageDescriptor,
}
impl Decoder for DynDecoder {
    type Item = DynamicMessage;
    type Error = Status;
    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Status> {
        let msg = DynamicMessage::decode(self.output.clone(), src)
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Some(msg))
    }
}

fn find_method(pool: &DescriptorPool, path: &str) -> Option<MethodDescriptor> {
    // path = "/package.Service/Method"
    let trimmed = path.trim_start_matches('/');
    let (service, method) = trimmed.rsplit_once('/')?;
    let svc = pool.get_service_by_name(service)?;
    let methods: Vec<MethodDescriptor> = svc.methods().collect();
    methods.into_iter().find(|m| m.name() == method)
}

/// Invoke `req.method` (unary or server-streaming). `allow_local` is the
/// workspace's explicit opt-in for local/private targets (grpc against a dev
/// server on localhost is a first-class API-client use case) — honoured by
/// reflection AND invoke alike.
pub async fn invoke(req: GrpcInvokeReq, allow_local: bool) -> Result<ApiResponse, Error> {
    let deadline = tokio::time::Instant::now() + GRPC_CALL_TIMEOUT;
    // Descriptors come from the uploaded .proto, or from server reflection when
    // none was provided.
    let pool = tokio::time::timeout_at(deadline, async {
        if req.proto.trim().is_empty() {
            reflect_pool_cached(&req.url, &req.headers, allow_local, false).await
        } else {
            pool_from_proto_cached(&req.proto).await
        }
    })
    .await
    .map_err(|_| upstream(deadline_status().to_string()))??;
    let method = find_method(&pool, &req.method)
        .ok_or_else(|| invalid(format!("method not found: {}", req.method)))?;
    if method.is_client_streaming() {
        return Err(invalid("client-streaming gRPC methods are not supported"));
    }
    let server_streaming = method.is_server_streaming();

    // Parse the JSON request body into a dynamic message.
    let input_desc = method.input();
    let body_text = if req.body.trim().is_empty() {
        "{}".to_string()
    } else {
        req.body.clone()
    };
    let mut de = serde_json::Deserializer::from_str(&body_text);
    let request_msg = DynamicMessage::deserialize(input_desc, &mut de)
        .map_err(|e| invalid(format!("request JSON does not match message: {e}")))?;

    let started = Instant::now();
    let mut trace = vec![TraceStep {
        label: "Request".into(),
        detail: format!("gRPC {} {}", req.url, req.method),
        ms: None,
        level: "info".into(),
    }];

    // SSRF guard (pinned) + channel build (TLS for https/grpcs) — reused
    // from the channel cache (the reflection above dialled it already).
    let channel = tokio::time::timeout_at(deadline, cached_channel(&req.url, allow_local))
        .await
        .map_err(|_| upstream(deadline_status().to_string()))??;
    trace.push(TraceStep {
        label: "Connected".into(),
        detail: req.url.clone(),
        ms: Some(started.elapsed().as_millis() as i64),
        level: "timing".into(),
    });

    let mut client = tonic::client::Grpc::new(channel);
    if let Err(e) = tokio::time::timeout_at(deadline, client.ready())
        .await
        .map_err(|_| upstream(deadline_status().to_string()))?
    {
        forget_channel(&req.url, allow_local);
        return Err(upstream(format!("not ready: {e}")));
    }

    let path = PathAndQuery::from_str(&req.method)
        .map_err(|e| invalid(format!("bad method path: {e}")))?;
    let mut request = tonic::Request::new(request_msg);
    for h in &req.headers {
        if h.key.trim().is_empty() || h.key.starts_with(':') {
            continue;
        }
        if let (Ok(name), Ok(val)) = (
            tonic::metadata::MetadataKey::from_bytes(h.key.to_ascii_lowercase().as_bytes()),
            tonic::metadata::MetadataValue::try_from(h.value.as_str()),
        ) {
            request.metadata_mut().insert(name, val);
        }
    }

    let codec = DynamicCodec {
        output: method.output(),
    };
    let call_started = Instant::now();

    // Unary → one message; server-streaming → a JSON array of messages. The
    // whole call is bounded by GRPC_CALL_TIMEOUT: a unary call that never
    // answers fails DEADLINE_EXCEEDED; a long-lived (watch-style) stream is
    // cut there — or at the message / byte cap — and returns what arrived,
    // flagged `truncated`.
    let outcome: Result<(Vec<Value>, String, bool), Status> = if server_streaming {
        let sent =
            match tokio::time::timeout_at(deadline, client.server_streaming(request, path, codec))
                .await
            {
                Err(_) => Err(deadline_status()),
                Ok(sent) => sent,
            };
        match sent {
            Ok(response) => {
                let meta = meta_to_json(response.metadata());
                let mut stream = response.into_inner();
                let mut collected = StreamCollector::default();
                let mut err: Option<Status> = None;
                loop {
                    match tokio::time::timeout_at(deadline, stream.message()).await {
                        Err(_) => {
                            collected.truncated = true;
                            break;
                        }
                        Ok(Ok(Some(m))) => {
                            if !collected.push(dynamic_to_value(&m)) {
                                break;
                            }
                        }
                        Ok(Ok(None)) => break,
                        Ok(Err(s)) => {
                            err = Some(s);
                            break;
                        }
                    }
                }
                match err {
                    Some(s) => Err(s),
                    None => Ok((
                        meta,
                        serde_json::to_string_pretty(&Value::Array(collected.msgs))
                            .unwrap_or_else(|_| "[]".into()),
                        collected.truncated,
                    )),
                }
            }
            Err(s) => Err(s),
        }
    } else {
        let sent = match tokio::time::timeout_at(deadline, client.unary(request, path, codec)).await
        {
            Err(_) => Err(deadline_status()),
            Ok(sent) => sent,
        };
        match sent {
            Ok(response) => {
                let meta = meta_to_json(response.metadata());
                Ok((meta, dynamic_to_json(&response.into_inner()), false))
            }
            Err(s) => Err(s),
        }
    };

    let call_ms = call_started.elapsed().as_millis() as i64;
    let duration_ms = started.elapsed().as_millis() as i64;

    match outcome {
        Ok((meta_headers, json_body, truncated)) => {
            let detail = if truncated {
                "stream cut at the time / size limit (partial)"
            } else if server_streaming {
                "stream complete"
            } else {
                "message received"
            };
            trace.push(TraceStep {
                label: "Response".into(),
                detail: detail.into(),
                ms: Some(call_ms),
                level: "timing".into(),
            });
            trace.push(TraceStep {
                label: "Completed".into(),
                detail: "OK (grpc-status 0)".into(),
                ms: Some(duration_ms),
                level: "success".into(),
            });
            let size = json_body.len() as i64;
            Ok(ApiResponse {
                status: 200,
                status_text: "OK".into(),
                headers: Value::Array(meta_headers),
                // `body` is the exact UTF-8 payload — no base64 copy needed.
                body_base64: String::new(),
                body_id: None,
                body: json_body,
                truncated,
                too_large: false,
                duration_ms,
                size_bytes: size,
                content_type: Some("application/grpc+json".into()),
                trace,
            })
        }
        Err(status) => {
            let code = status.code();
            trace.push(TraceStep {
                label: "Completed".into(),
                detail: format!("grpc-status {} {:?}", code as i32, code),
                ms: Some(duration_ms),
                level: "error".into(),
            });
            let body = json!({
                "grpc_status": code as i32,
                "grpc_status_name": format!("{code:?}"),
                "message": status.message(),
            })
            .to_string();
            Ok(ApiResponse {
                status: 500,
                status_text: format!("gRPC {code:?}"),
                headers: Value::Array(vec![
                    json!({"key":"grpc-status","value": (code as i32).to_string()}),
                ]),
                body_base64: String::new(),
                body_id: None,
                size_bytes: body.len() as i64,
                body,
                truncated: false,
                too_large: false,
                duration_ms,
                content_type: Some("application/grpc+json".into()),
                trace,
            })
        }
    }
}

fn meta_to_json(meta: &tonic::metadata::MetadataMap) -> Vec<Value> {
    meta.clone()
        .into_headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|vs| json!({ "key": k.as_str(), "value": vs }))
        })
        .collect()
}

fn dynamic_to_value(msg: &DynamicMessage) -> Value {
    serde_json::to_value(msg).unwrap_or(Value::Null)
}

// ── server reflection ────────────────────────────────────────────────────────

/// Minimal gRPC server-reflection proto (v1alpha) — compiled at runtime so we
/// can call the reflection service dynamically without generated stubs.
const REFLECTION_PROTO: &str = r#"
syntax = "proto3";
package grpc.reflection.v1alpha;
service ServerReflection {
  rpc ServerReflectionInfo(stream ServerReflectionRequest) returns (stream ServerReflectionResponse);
}
message ServerReflectionRequest {
  string host = 1;
  oneof message_request {
    string file_by_filename = 3;
    string file_containing_symbol = 4;
    string list_services = 7;
  }
}
message ServerReflectionResponse {
  string valid_host = 1;
  ServerReflectionRequest original_request = 2;
  oneof message_response {
    FileDescriptorResponse file_descriptor_response = 4;
    ListServiceResponse list_services_response = 6;
    ErrorResponse error_response = 7;
  }
}
message FileDescriptorResponse { repeated bytes file_descriptor_proto = 1; }
message ListServiceResponse { repeated ServiceResponse service = 1; }
message ServiceResponse { string name = 1; }
message ErrorResponse { int32 error_code = 1; string error_message = 2; }
"#;

/// Build the tonic endpoint for `url` (TLS for `https`/`grpcs`).
///
/// SSRF guard: unless the workspace opted in to local/private targets, the host
/// is resolved + vetted ONCE and the channel dials exactly that vetted address
/// (pinned) — handing tonic the hostname would resolve it again at connect
/// time, re-opening DNS rebinding. The original authority is kept as the
/// request origin (`:authority`) and as the TLS server name, so virtual hosts
/// and certificate verification behave exactly as for the hostname.
async fn grpc_endpoint(url: &str, allow_local: bool) -> Result<Endpoint, Error> {
    let uri = http::Uri::from_str(url).map_err(|e| invalid(format!("bad url: {e}")))?;
    let is_tls = matches!(uri.scheme_str(), Some("https") | Some("grpcs"));
    let mut endpoint = if allow_local {
        Channel::builder(uri.clone())
    } else {
        let (host, addrs) = otto_netguard::resolve_checked(url).await.map_err(invalid)?;
        let mut addr = addrs
            .iter()
            .find(|a| a.is_ipv4())
            .or_else(|| addrs.first())
            .copied()
            .ok_or_else(|| invalid(format!("host {host} did not resolve")))?;
        if uri.port_u16().is_none() {
            // `grpc`/`grpcs` have no registered default port in the URL parser.
            addr.set_port(if is_tls { 443 } else { 80 });
        }
        // tonic only layers TLS on an `https` connect URI, so normalise the
        // scheme (`grpc`/`grpcs` included) for both the dial and the origin.
        let scheme = if is_tls { "https" } else { "http" };
        let authority = uri
            .authority()
            .map(|a| a.as_str().to_string())
            .ok_or_else(|| invalid("url has no host"))?;
        let pinned = http::Uri::from_str(&format!("{scheme}://{addr}"))
            .map_err(|e| invalid(format!("bad url: {e}")))?;
        let origin = http::Uri::from_str(&format!("{scheme}://{authority}"))
            .map_err(|e| invalid(format!("bad url: {e}")))?;
        let mut endpoint = Channel::builder(pinned).origin(origin);
        if is_tls {
            let tls = ClientTlsConfig::new().with_webpki_roots().domain_name(host);
            endpoint = endpoint
                .tls_config(tls)
                .map_err(|e| upstream(format!("tls config: {e}")))?;
        }
        return Ok(endpoint);
    };
    if is_tls {
        let tls = ClientTlsConfig::new().with_webpki_roots();
        endpoint = endpoint
            .tls_config(tls)
            .map_err(|e| upstream(format!("tls config: {e}")))?;
    }
    Ok(endpoint)
}

async fn connect_channel(url: &str, allow_local: bool) -> Result<Channel, Error> {
    // SSRF guard (pinned, see `grpc_endpoint`) for the reflection target —
    // honouring the workspace allow-local opt-in exactly like invoke does.
    let endpoint = grpc_endpoint(url, allow_local).await?;
    CHANNEL_CONNECTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    match tokio::time::timeout(Duration::from_secs(20), endpoint.connect()).await {
        Ok(Ok(c)) => Ok(c),
        Ok(Err(e)) => Err(upstream(format!("connect failed: {e}"))),
        Err(_) => Err(upstream("connection timed out")),
    }
}

fn metadata_from(headers: &[KV]) -> tonic::metadata::MetadataMap {
    let mut md = tonic::metadata::MetadataMap::new();
    for h in headers {
        if h.key.trim().is_empty() || h.key.starts_with(':') {
            continue;
        }
        if let (Ok(k), Ok(v)) = (
            tonic::metadata::MetadataKey::from_bytes(h.key.to_ascii_lowercase().as_bytes()),
            tonic::metadata::MetadataValue::try_from(h.value.as_str()),
        ) {
            md.insert(k, v);
        }
    }
    md
}

/// Build a descriptor pool by querying the server's reflection service.
async fn reflect_pool(
    url: &str,
    headers: &[KV],
    allow_local: bool,
) -> Result<DescriptorPool, Error> {
    tokio::time::timeout(
        Duration::from_secs(20),
        reflect_pool_inner(url, headers, allow_local),
    )
    .await
    .map_err(|_| upstream("reflection timed out after 20s"))?
}

/// Bound the cumulative response, including duplicate/irrelevant messages:
/// tonic's per-message decode cap alone cannot bound a streaming RPC.
#[derive(Default)]
struct ReflectionBudget {
    messages: usize,
    bytes: usize,
}
impl ReflectionBudget {
    fn account(&mut self, message: &DynamicMessage) -> Result<(), Error> {
        self.messages += 1;
        self.bytes = self.bytes.saturating_add(message.encoded_len());
        if self.messages > 2048 || self.bytes > 8 * 1024 * 1024 {
            return Err(upstream(
                "reflection response limit exceeded (2048 messages / 8 MiB)",
            ));
        }
        Ok(())
    }
}

async fn reflect_pool_inner(
    url: &str,
    headers: &[KV],
    allow_local: bool,
) -> Result<DescriptorPool, Error> {
    use futures_util::stream;
    use prost::Message as _;
    use prost_reflect::Value as PValue;

    let refl = pool_from_proto_cached(REFLECTION_PROTO).await?;
    let req_desc = refl
        .get_message_by_name("grpc.reflection.v1alpha.ServerReflectionRequest")
        .ok_or_else(|| upstream("reflection request descriptor missing"))?;
    let resp_desc = refl
        .get_message_by_name("grpc.reflection.v1alpha.ServerReflectionResponse")
        .ok_or_else(|| upstream("reflection response descriptor missing"))?;
    let path =
        PathAndQuery::from_static("/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo");

    let channel = cached_channel(url, allow_local).await?;
    let mut client = tonic::client::Grpc::new(channel);
    if let Err(e) = client.ready().await {
        forget_channel(url, allow_local);
        return Err(upstream(format!("not ready: {e}")));
    }
    let md = metadata_from(headers);

    let make_req = |field: &str, value: &str| -> DynamicMessage {
        let mut m = DynamicMessage::new(req_desc.clone());
        m.set_field_by_name(field, PValue::String(value.to_string()));
        m
    };

    // 1. list services
    let mut req1 = tonic::Request::new(stream::iter(vec![make_req("list_services", "*")]));
    *req1.metadata_mut() = md.clone();
    let resp1 = client
        .streaming(
            req1,
            path.clone(),
            DynamicCodec {
                output: resp_desc.clone(),
            },
        )
        .await
        .map_err(|s| upstream(format!("reflection unavailable: {}", s.message())))?;
    let mut s1 = resp1.into_inner();
    let mut services: Vec<String> = Vec::new();
    let mut service_names = std::collections::HashSet::new();
    let mut budget = ReflectionBudget::default();
    while let Some(m) = s1
        .message()
        .await
        .map_err(|s| upstream(format!("reflection listing failed: {}", s.message())))?
    {
        budget.account(&m)?;
        if let Some(lsr) = m.get_field_by_name("list_services_response") {
            if let Some(list) = lsr
                .as_message()
                .and_then(|x| x.get_field_by_name("service"))
            {
                if let Some(arr) = list.as_list() {
                    for sv in arr {
                        if let Some(name) = sv
                            .as_message()
                            .and_then(|x| x.get_field_by_name("name"))
                            .and_then(|v| v.as_str().map(String::from))
                        {
                            if !name.is_empty()
                                && !name.starts_with("grpc.reflection")
                                && service_names.insert(name.clone())
                            {
                                if services.len() >= 1024 {
                                    return Err(upstream(
                                        "reflection service limit exceeded (1024 services)",
                                    ));
                                }
                                services.push(name);
                            }
                        }
                    }
                }
            }
        }
    }
    if services.is_empty() {
        return Err(upstream(
            "server returned no services (reflection may be disabled)",
        ));
    }

    // 2. fetch the file descriptors containing each service
    let reqs: Vec<DynamicMessage> = services
        .iter()
        .map(|s| make_req("file_containing_symbol", s))
        .collect();
    let mut req2 = tonic::Request::new(stream::iter(reqs));
    *req2.metadata_mut() = md;
    // Each RPC consumes tonic's readiness reservation. Reserve again before
    // the second call; otherwise a healthy reflection server can panic here.
    client
        .ready()
        .await
        .map_err(|e| upstream(format!("reflection not ready: {e}")))?;
    let resp2 = client
        .streaming(req2, path, DynamicCodec { output: resp_desc })
        .await
        .map_err(|s| upstream(format!("reflection fetch failed: {}", s.message())))?;
    let mut s2 = resp2.into_inner();
    let mut files: Vec<prost_types::FileDescriptorProto> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    while let Some(m) = s2
        .message()
        .await
        .map_err(|s| upstream(format!("reflection descriptors failed: {}", s.message())))?
    {
        budget.account(&m)?;
        if let Some(fdr) = m.get_field_by_name("file_descriptor_response") {
            if let Some(list) = fdr
                .as_message()
                .and_then(|x| x.get_field_by_name("file_descriptor_proto"))
            {
                if let Some(arr) = list.as_list() {
                    for b in arr {
                        if let Some(bytes) = b.as_bytes() {
                            if let Ok(fdp) = prost_types::FileDescriptorProto::decode(bytes.clone())
                            {
                                let name = fdp.name.clone().unwrap_or_default();
                                if seen.insert(name) {
                                    if files.len() >= 4096 {
                                        return Err(upstream(
                                            "reflection file limit exceeded (4096 descriptors)",
                                        ));
                                    }
                                    files.push(fdp);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if files.is_empty() {
        return Err(upstream("no file descriptors returned by reflection"));
    }
    let fds = prost_types::FileDescriptorSet { file: files };
    let mut buf = Vec::new();
    fds.encode(&mut buf)
        .map_err(|e| upstream(format!("encode descriptors: {e}")))?;
    DescriptorPool::decode(buf.as_slice()).map_err(|e| invalid(format!("descriptor build: {e}")))
}

#[derive(Deserialize)]
pub struct GrpcReflectReq {
    url: String,
    #[serde(default)]
    headers: Vec<KV>,
}

/// `POST /workspaces/{wid}/api-client/grpc/reflect` — list services/methods via
/// the server's reflection API (no .proto upload).
/// Always re-reflects (`fresh`), refreshing the cached pool.
pub async fn reflect(req: &GrpcReflectReq, allow_local: bool) -> Result<GrpcDescribeResp, Error> {
    let pool = reflect_pool_cached(&req.url, &req.headers, allow_local, true).await?;
    Ok(GrpcDescribeResp {
        services: services_from_pool(&pool),
    })
}

fn dynamic_to_json(msg: &DynamicMessage) -> String {
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::pretty(&mut buf);
    match msg.serialize(&mut ser) {
        Ok(_) => String::from_utf8(buf).unwrap_or_else(|_| "{}".to_string()),
        Err(e) => json!({ "error": format!("serialize response: {e}") }).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_collector_stops_at_the_message_and_byte_caps() {
        let mut c = StreamCollector::default();
        for i in 0..GRPC_STREAM_MAX_MSGS {
            assert!(c.push(json!({ "i": i })));
        }
        assert!(!c.truncated);
        assert!(!c.push(json!({ "i": "one too many" })));
        assert!(c.truncated);
        assert_eq!(c.msgs.len(), GRPC_STREAM_MAX_MSGS);

        let mut c = StreamCollector::default();
        let big = "x".repeat(GRPC_STREAM_MAX_BYTES / 2);
        assert!(c.push(json!(big)));
        assert!(
            !c.push(json!(big)),
            "second half-cap message overflows the byte cap"
        );
        assert!(c.truncated);
        assert_eq!(c.msgs.len(), 1);
    }

    const SAMPLE: &str = r#"
        syntax = "proto3";
        package demo;
        message HelloRequest { string name = 1; int32 count = 2; }
        message HelloReply { string message = 1; repeated string tags = 3; }
        service Greeter {
          rpc SayHello (HelloRequest) returns (HelloReply);
        }
    "#;

    /// Perf guard (F6/F11): a .proto is compiled once per content (on the
    /// blocking pool) and the TTL cache is bounded and expires.
    #[tokio::test]
    async fn proto_pool_is_compiled_once_per_content() {
        let src = format!("{SAMPLE}\n// cache-test {}", otto_core::new_id());
        let before = PROTO_COMPILES.load(std::sync::atomic::Ordering::Relaxed);
        let a = pool_from_proto_cached(&src).await.unwrap();
        let b = pool_from_proto_cached(&src).await.unwrap();
        assert_eq!(a.services().count(), b.services().count());
        // Other tests compile concurrently, so count only our own misses:
        // a third call on the same content must not compile again.
        let after_two = PROTO_COMPILES.load(std::sync::atomic::Ordering::Relaxed);
        assert!(after_two > before);
        assert!(lock(pool_cache())
            .get(&format!("proto:{}", sha_key(&[&src])))
            .is_some());

        let mut c: TtlCache<u32> = TtlCache::new(Duration::from_millis(30), 2);
        c.put("a".into(), 1);
        c.put("b".into(), 2);
        c.put("c".into(), 3);
        assert_eq!(c.map.len(), 2, "bounded");
        std::thread::sleep(Duration::from_millis(50));
        assert!(c.get("c").is_none(), "expired");
        assert_ne!(
            reflect_key("https://a", &[], false),
            reflect_key("https://a", &[], true),
            "allow-local is part of the key"
        );
    }

    #[test]
    fn describes_services_and_methods() {
        let pool = pool_from_proto(SAMPLE).expect("compile proto");
        let svc = pool.get_service_by_name("demo.Greeter").expect("service");
        let methods: Vec<_> = svc.methods().collect();
        assert_eq!(methods.len(), 1);
        let m = &methods[0];
        assert_eq!(m.name(), "SayHello");
        assert_eq!(m.input().full_name(), "demo.HelloRequest");
        assert_eq!(m.output().full_name(), "demo.HelloReply");
        assert!(!m.is_client_streaming() && !m.is_server_streaming());
    }

    async fn reflection_peer(
        message: Option<DynamicMessage>,
        repeats: usize,
    ) -> (String, tokio::task::JoinHandle<()>) {
        reflection_peer_responses(vec![message], repeats).await
    }

    async fn reflection_peer_responses(
        messages: Vec<Option<DynamicMessage>>,
        repeats: usize,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut connection = h2::server::handshake(socket).await.unwrap();
            let mut held = Vec::new();
            let mut messages = messages.into_iter();
            while let Some(Ok((request, mut reply))) = connection.accept().await {
                let response = http::Response::builder()
                    .status(200)
                    .header("content-type", "application/grpc")
                    .body(())
                    .unwrap();
                let mut body = reply.send_response(response, false).unwrap();
                if let Some(Some(message)) = messages.next() {
                    let encoded = message.encode_to_vec();
                    let mut frame = vec![0];
                    frame.extend_from_slice(&(encoded.len() as u32).to_be_bytes());
                    frame.extend_from_slice(&encoded);
                    for _ in 0..repeats {
                        body.send_data(frame.clone().into(), false).unwrap();
                    }
                    let mut trailers = http::HeaderMap::new();
                    trailers.insert("grpc-status", http::HeaderValue::from_static("0"));
                    body.send_trailers(trailers).unwrap();
                }
                held.push((request, body));
            }
        });
        (url, server)
    }

    #[tokio::test]
    async fn reflection_reserves_readiness_for_descriptor_rpc() {
        use prost_reflect::Value as PValue;
        let reflection = pool_from_proto(REFLECTION_PROTO).unwrap();
        let response = reflection
            .get_message_by_name("grpc.reflection.v1alpha.ServerReflectionResponse")
            .unwrap();
        let mut listing_json = serde_json::Deserializer::from_str(
            r#"{"listServicesResponse":{"service":[{"name":"demo.Greeter"}]}}"#,
        );
        let listing = DynamicMessage::deserialize(response.clone(), &mut listing_json).unwrap();
        let mut descriptors = DynamicMessage::new(
            reflection
                .get_message_by_name("grpc.reflection.v1alpha.FileDescriptorResponse")
                .unwrap(),
        );
        let sample = pool_from_proto(SAMPLE).unwrap();
        descriptors.set_field_by_name(
            "file_descriptor_proto",
            PValue::List(
                sample
                    .files()
                    .map(|f| PValue::Bytes(f.file_descriptor_proto().encode_to_vec().into()))
                    .collect(),
            ),
        );
        let mut descriptor_response = DynamicMessage::new(response);
        descriptor_response
            .set_field_by_name("file_descriptor_response", PValue::Message(descriptors));
        let (url, server) =
            reflection_peer_responses(vec![Some(listing), Some(descriptor_response)], 1).await;
        let result =
            tokio::time::timeout(Duration::from_secs(5), reflect_pool(&url, &[], true)).await;
        server.abort();
        forget_channel(&url, true);
        let reflected = result.unwrap().unwrap();
        assert!(reflected.get_service_by_name("demo.Greeter").is_some());
    }

    #[tokio::test]
    async fn reflection_deadline_covers_a_server_that_never_finishes() {
        let (url, server) = reflection_peer(None, 0).await;
        let result =
            tokio::time::timeout(Duration::from_secs(22), reflect_pool(&url, &[], true)).await;
        server.abort();
        forget_channel(&url, true);
        let error = result
            .expect("reflection must have its own deadline")
            .unwrap_err();
        assert!(
            error.to_string().contains("reflection timed out"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn reflection_rejects_excessive_service_listing() {
        let pool = pool_from_proto(REFLECTION_PROTO).unwrap();
        let descriptor = pool
            .get_message_by_name("grpc.reflection.v1alpha.ServerReflectionResponse")
            .unwrap();
        let body = json!({"listServicesResponse":{"service":(0..1025).map(|n| json!({"name":format!("service{n}")})).collect::<Vec<_>>()}}).to_string();
        let message =
            DynamicMessage::deserialize(descriptor, &mut serde_json::Deserializer::from_str(&body))
                .unwrap();
        let (url, server) = reflection_peer(Some(message), 1).await;
        let result =
            tokio::time::timeout(Duration::from_secs(5), reflect_pool(&url, &[], true)).await;
        server.abort();
        forget_channel(&url, true);
        let error = result.unwrap().unwrap_err();
        assert!(error.to_string().contains("service limit"), "{error}");
    }

    #[tokio::test]
    async fn reflection_rejects_excessive_bytes_and_message_counts() {
        let pool = pool_from_proto(REFLECTION_PROTO).unwrap();
        let descriptor = pool
            .get_message_by_name("grpc.reflection.v1alpha.ServerReflectionResponse")
            .unwrap();
        for (name, count) in [
            ("grpc.reflection".to_string(), 2049),
            (format!("grpc.reflection{}", "x".repeat(1024 * 1024)), 9),
        ] {
            let body = json!({"listServicesResponse":{"service":[{"name":name}]}}).to_string();
            let message = DynamicMessage::deserialize(
                descriptor.clone(),
                &mut serde_json::Deserializer::from_str(&body),
            )
            .unwrap();
            let (url, server) = reflection_peer(Some(message), count).await;
            let result =
                tokio::time::timeout(Duration::from_secs(5), reflect_pool(&url, &[], true)).await;
            server.abort();
            forget_channel(&url, true);
            let error = result.unwrap().unwrap_err();
            assert!(error.to_string().contains("response limit"), "{error}");
        }
    }

    #[test]
    fn json_skeleton_has_fields_with_defaults() {
        let pool = pool_from_proto(SAMPLE).expect("compile proto");
        let req = pool.get_message_by_name("demo.HelloRequest").expect("msg");
        let skeleton: Value = serde_json::from_str(&json_skeleton(&req, 0)).unwrap();
        assert_eq!(skeleton["name"], json!(""));
        assert_eq!(skeleton["count"], json!(0));
    }

    #[tokio::test]
    async fn endpoint_pins_vetted_address_and_honours_allow_local() {
        // `grpcs://` builds a TLS config, which needs the process-level rustls
        // provider that `ottod`'s main installs at startup; a test binary must
        // install it itself (idempotent — `Err` just means it's already set).
        let _ = rustls::crypto::ring::default_provider().install_default();
        // Guarded: a loopback target is refused (reflection included).
        assert!(grpc_endpoint("grpc://127.0.0.1:50051", false)
            .await
            .is_err());
        assert!(grpc_endpoint("http://[::1]:50051", false).await.is_err());
        // allow_local: the workspace opt-in lets a local dev server through.
        assert!(grpc_endpoint("grpc://127.0.0.1:50051", true).await.is_ok());
        // A public literal is dialled at the vetted address, scheme normalised.
        let ep = grpc_endpoint("grpc://8.8.8.8:50051", false).await.unwrap();
        assert_eq!(ep.uri().scheme_str(), Some("http"));
        assert_eq!(ep.uri().host(), Some("8.8.8.8"));
        assert_eq!(ep.uri().port_u16(), Some(50051));
        let ep = grpc_endpoint("grpcs://8.8.8.8", false).await.unwrap();
        assert_eq!(ep.uri().scheme_str(), Some("https"));
        assert_eq!(ep.uri().port_u16(), Some(443));
    }

    #[test]
    fn find_method_resolves_call_path() {
        let pool = pool_from_proto(SAMPLE).expect("compile proto");
        let m = find_method(&pool, "/demo.Greeter/SayHello").expect("method");
        assert_eq!(m.name(), "SayHello");
        assert!(find_method(&pool, "/demo.Greeter/Nope").is_none());
    }

    #[test]
    fn reflection_proto_compiles_and_builds_request() {
        use prost::Message as _;
        let pool = pool_from_proto(REFLECTION_PROTO).expect("compile reflection proto");
        let desc = pool
            .get_message_by_name("grpc.reflection.v1alpha.ServerReflectionRequest")
            .expect("request descriptor");
        let mut m = DynamicMessage::new(desc.clone());
        m.set_field_by_name("list_services", prost_reflect::Value::String("*".into()));
        let mut buf = Vec::new();
        m.encode(&mut buf).unwrap();
        let back = DynamicMessage::decode(desc, buf.as_slice()).unwrap();
        assert_eq!(
            back.get_field_by_name("list_services")
                .and_then(|v| v.as_str().map(String::from)),
            Some("*".to_string())
        );
        // The reflection service itself is filtered out of results.
        assert!(services_from_pool(&pool).is_empty());
    }

    #[test]
    fn round_trips_dynamic_message_json() {
        let pool = pool_from_proto(SAMPLE).expect("compile proto");
        let desc = pool.get_message_by_name("demo.HelloRequest").expect("msg");
        let mut de = serde_json::Deserializer::from_str(r#"{"name":"otto","count":3}"#);
        let msg = DynamicMessage::deserialize(desc, &mut de).expect("decode json");
        // encode to protobuf bytes then back
        let mut bytes = Vec::new();
        msg.encode(&mut bytes).unwrap();
        let desc2 = pool.get_message_by_name("demo.HelloRequest").unwrap();
        let decoded = DynamicMessage::decode(desc2, bytes.as_slice()).unwrap();
        let json = dynamic_to_json(&decoded);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["name"], json!("otto"));
        assert_eq!(v["count"], json!(3));
    }

    /// Perf guard (N5): invoke and reflection share one dialled channel per
    /// `(url, allow_local)` — repeat calls reuse it (one TCP connection at a
    /// real h2 server), and only a transport failure (`forget_channel`)
    /// makes the next call redial.
    #[tokio::test]
    async fn grpc_channel_is_dialled_once_and_redialled_after_forget() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepts = Arc::new(AtomicUsize::new(0));
        let counted = accepts.clone();
        // A minimal HTTP/2 (h2c prior-knowledge) server: completes the
        // handshake and holds the connection, like any gRPC server would.
        let server = tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else {
                    return;
                };
                counted.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    if let Ok(mut conn) = h2::server::handshake(sock).await {
                        while let Some(Ok(_)) = conn.accept().await {}
                    }
                });
            }
        });
        let url = format!("http://{addr}");
        let dials = || CHANNEL_CONNECTS.load(Ordering::Relaxed);
        let before = dials();
        let a = cached_channel(&url, true).await.expect("dial");
        let _b = cached_channel(&url, true).await.expect("reuse");
        let _c = cached_channel(&url, true).await.expect("reuse");
        let mut client = tonic::client::Grpc::new(a);
        client.ready().await.expect("ready");
        assert_eq!(dials() - before, 1, "three calls, one dial");
        for _ in 0..100 {
            if accepts.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(accepts.load(Ordering::SeqCst), 1, "one TCP connection");

        // A transport failure drops the cached channel; the next call redials.
        forget_channel(&url, true);
        let _d = cached_channel(&url, true).await.expect("redial");
        assert_eq!(dials() - before, 2);
        for _ in 0..100 {
            if accepts.load(Ordering::SeqCst) >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(accepts.load(Ordering::SeqCst), 2);
        forget_channel(&url, true);
        server.abort();
    }
}
