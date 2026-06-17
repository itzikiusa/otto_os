//! gRPC / protobuf endpoints for the API client:
//!   POST /workspaces/{wid}/api-client/grpc/describe  — parse a `.proto`, list
//!        services/methods + a JSON request skeleton (viewable descriptors).
//!   POST /workspaces/{wid}/api-client/grpc/invoke    — dynamically invoke a
//!        unary method (JSON in → JSON out) through a real gRPC connection.
//!
//! Parsing uses `protox` (no `protoc` needed); dynamic messages use
//! `prost-reflect`; transport uses `tonic` with a descriptor-driven codec.

use std::str::FromStr;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::uri::PathAndQuery;
use axum::Json;
use prost::Message as _;
use prost_reflect::{DescriptorPool, DynamicMessage, Kind, MessageDescriptor, MethodDescriptor};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::transport::{Channel, ClientTlsConfig};
use tonic::Status;

use otto_core::api::{ApiResponse, TraceStep};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

fn invalid(msg: impl Into<String>) -> ApiError {
    ApiError(Error::Invalid(msg.into()))
}
fn upstream(msg: impl Into<String>) -> ApiError {
    ApiError(Error::Upstream(msg.into()))
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
fn pool_from_proto(proto: &str) -> Result<DescriptorPool, ApiError> {
    if proto.trim().is_empty() {
        return Err(invalid("empty .proto"));
    }
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

pub async fn describe(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<GrpcDescribeReq>,
) -> ApiResult<Json<GrpcDescribeResp>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let pool = pool_from_proto(&req.proto)?;
    let services = services_from_pool(&pool);
    Ok(Json(GrpcDescribeResp { services }))
}

/// Build a JSON skeleton for a message (depth-limited to avoid recursion blowups).
fn json_skeleton(msg: &MessageDescriptor, depth: u8) -> String {
    serde_json::to_string_pretty(&skeleton_value(msg, depth)).unwrap_or_else(|_| "{}".to_string())
}

fn skeleton_value(msg: &MessageDescriptor, depth: u8) -> Value {
    let mut map = serde_json::Map::new();
    for field in msg.fields() {
        map.insert(field.json_name().to_string(), field_placeholder(&field, depth));
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
        Kind::Double | Kind::Float | Kind::Int32 | Kind::Int64 | Kind::Uint32 | Kind::Uint64
        | Kind::Sint32 | Kind::Sint64 | Kind::Fixed32 | Kind::Fixed64 | Kind::Sfixed32
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
        item.encode(dst).map_err(|e| Status::internal(e.to_string()))
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

pub async fn invoke(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<GrpcInvokeReq>,
) -> ApiResult<Json<ApiResponse>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;

    // Descriptors come from the uploaded .proto, or from server reflection when
    // none was provided.
    let pool = if req.proto.trim().is_empty() {
        reflect_pool(&req.url, &req.headers).await?
    } else {
        pool_from_proto(&req.proto)?
    };
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
    let mut trace = vec![
        TraceStep { label: "Request".into(), detail: format!("gRPC {} {}", req.url, req.method), ms: None, level: "info".into() },
    ];

    // Build the channel (TLS for https/grpcs).
    let uri = axum::http::Uri::from_str(&req.url).map_err(|e| invalid(format!("bad url: {e}")))?;
    let scheme = uri.scheme_str().unwrap_or("http");
    let mut endpoint = Channel::builder(uri.clone());
    if scheme == "https" || scheme == "grpcs" {
        let tls = ClientTlsConfig::new().with_webpki_roots();
        endpoint = endpoint
            .tls_config(tls)
            .map_err(|e| upstream(format!("tls config: {e}")))?;
    }
    let connect = tokio::time::timeout(Duration::from_secs(20), endpoint.connect()).await;
    let channel = match connect {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return Err(upstream(format!("connect failed: {e}"))),
        Err(_) => return Err(upstream("connection timed out")),
    };
    trace.push(TraceStep { label: "Connected".into(), detail: req.url.clone(), ms: Some(started.elapsed().as_millis() as i64), level: "timing".into() });

    let mut client = tonic::client::Grpc::new(channel);
    client.ready().await.map_err(|e| upstream(format!("not ready: {e}")))?;

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

    let codec = DynamicCodec { output: method.output() };
    let call_started = Instant::now();

    // Unary → one message; server-streaming → a JSON array of messages.
    let outcome: Result<(Vec<Value>, String), Status> = if server_streaming {
        match client.server_streaming(request, path, codec).await {
            Ok(response) => {
                let meta = meta_to_json(response.metadata());
                let mut stream = response.into_inner();
                let mut msgs: Vec<Value> = Vec::new();
                let mut err: Option<Status> = None;
                loop {
                    match stream.message().await {
                        Ok(Some(m)) => msgs.push(dynamic_to_value(&m)),
                        Ok(None) => break,
                        Err(s) => {
                            err = Some(s);
                            break;
                        }
                    }
                }
                match err {
                    Some(s) => Err(s),
                    None => Ok((
                        meta,
                        serde_json::to_string_pretty(&Value::Array(msgs)).unwrap_or_else(|_| "[]".into()),
                    )),
                }
            }
            Err(s) => Err(s),
        }
    } else {
        match client.unary(request, path, codec).await {
            Ok(response) => {
                let meta = meta_to_json(response.metadata());
                Ok((meta, dynamic_to_json(&response.into_inner())))
            }
            Err(s) => Err(s),
        }
    };

    let call_ms = call_started.elapsed().as_millis() as i64;
    let duration_ms = started.elapsed().as_millis() as i64;
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;

    match outcome {
        Ok((meta_headers, json_body)) => {
            let detail = if server_streaming { "stream complete" } else { "message received" };
            trace.push(TraceStep { label: "Response".into(), detail: detail.into(), ms: Some(call_ms), level: "timing".into() });
            trace.push(TraceStep { label: "Completed".into(), detail: "OK (grpc-status 0)".into(), ms: Some(duration_ms), level: "success".into() });
            let size = json_body.len() as i64;
            Ok(Json(ApiResponse {
                status: 200,
                status_text: "OK".into(),
                headers: Value::Array(meta_headers),
                body_base64: B64.encode(json_body.as_bytes()),
                body: json_body,
                truncated: false,
                too_large: false,
                duration_ms,
                size_bytes: size,
                content_type: Some("application/grpc+json".into()),
                trace,
            }))
        }
        Err(status) => {
            let code = status.code();
            trace.push(TraceStep { label: "Completed".into(), detail: format!("grpc-status {} {:?}", code as i32, code), ms: Some(duration_ms), level: "error".into() });
            let body = json!({
                "grpc_status": code as i32,
                "grpc_status_name": format!("{code:?}"),
                "message": status.message(),
            })
            .to_string();
            Ok(Json(ApiResponse {
                status: 500,
                status_text: format!("gRPC {code:?}"),
                headers: Value::Array(vec![json!({"key":"grpc-status","value": (code as i32).to_string()})]),
                body_base64: B64.encode(body.as_bytes()),
                size_bytes: body.len() as i64,
                body,
                truncated: false,
                too_large: false,
                duration_ms,
                content_type: Some("application/grpc+json".into()),
                trace,
            }))
        }
    }
}

fn meta_to_json(meta: &tonic::metadata::MetadataMap) -> Vec<Value> {
    meta.clone()
        .into_headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|vs| json!({ "key": k.as_str(), "value": vs })))
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

async fn connect_channel(url: &str) -> Result<Channel, ApiError> {
    let uri = axum::http::Uri::from_str(url).map_err(|e| invalid(format!("bad url: {e}")))?;
    let is_tls = matches!(uri.scheme_str(), Some("https") | Some("grpcs"));
    let mut endpoint = Channel::builder(uri);
    if is_tls {
        let tls = ClientTlsConfig::new().with_webpki_roots();
        endpoint = endpoint
            .tls_config(tls)
            .map_err(|e| upstream(format!("tls config: {e}")))?;
    }
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
async fn reflect_pool(url: &str, headers: &[KV]) -> Result<DescriptorPool, ApiError> {
    use futures_util::stream;
    use prost::Message as _;
    use prost_reflect::Value as PValue;

    let refl = pool_from_proto(REFLECTION_PROTO)?;
    let req_desc = refl
        .get_message_by_name("grpc.reflection.v1alpha.ServerReflectionRequest")
        .ok_or_else(|| upstream("reflection request descriptor missing"))?;
    let resp_desc = refl
        .get_message_by_name("grpc.reflection.v1alpha.ServerReflectionResponse")
        .ok_or_else(|| upstream("reflection response descriptor missing"))?;
    let path = PathAndQuery::from_static(
        "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo",
    );

    let channel = connect_channel(url).await?;
    let mut client = tonic::client::Grpc::new(channel);
    client
        .ready()
        .await
        .map_err(|e| upstream(format!("not ready: {e}")))?;
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
        .streaming(req1, path.clone(), DynamicCodec { output: resp_desc.clone() })
        .await
        .map_err(|s| upstream(format!("reflection unavailable: {}", s.message())))?;
    let mut s1 = resp1.into_inner();
    let mut services: Vec<String> = Vec::new();
    while let Ok(Some(m)) = s1.message().await {
        if let Some(lsr) = m.get_field_by_name("list_services_response") {
            if let Some(list) = lsr.as_message().and_then(|x| x.get_field_by_name("service")) {
                if let Some(arr) = list.as_list() {
                    for sv in arr {
                        if let Some(name) = sv.as_message()
                            .and_then(|x| x.get_field_by_name("name"))
                            .and_then(|v| v.as_str().map(String::from))
                        {
                            services.push(name);
                        }
                    }
                }
            }
        }
    }
    services.retain(|s| !s.is_empty() && !s.starts_with("grpc.reflection"));
    if services.is_empty() {
        return Err(upstream("server returned no services (reflection may be disabled)"));
    }

    // 2. fetch the file descriptors containing each service
    let reqs: Vec<DynamicMessage> = services
        .iter()
        .map(|s| make_req("file_containing_symbol", s))
        .collect();
    let mut req2 = tonic::Request::new(stream::iter(reqs));
    *req2.metadata_mut() = md;
    let resp2 = client
        .streaming(req2, path, DynamicCodec { output: resp_desc })
        .await
        .map_err(|s| upstream(format!("reflection fetch failed: {}", s.message())))?;
    let mut s2 = resp2.into_inner();
    let mut files: Vec<prost_types::FileDescriptorProto> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    while let Ok(Some(m)) = s2.message().await {
        if let Some(fdr) = m.get_field_by_name("file_descriptor_response") {
            if let Some(list) = fdr.as_message().and_then(|x| x.get_field_by_name("file_descriptor_proto")) {
                if let Some(arr) = list.as_list() {
                    for b in arr {
                        if let Some(bytes) = b.as_bytes() {
                            if let Ok(fdp) = prost_types::FileDescriptorProto::decode(bytes.clone()) {
                                let name = fdp.name.clone().unwrap_or_default();
                                if seen.insert(name) {
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
    DescriptorPool::decode(buf.as_slice())
        .map_err(|e| invalid(format!("descriptor build: {e}")))
}

#[derive(Deserialize)]
pub struct GrpcReflectReq {
    url: String,
    #[serde(default)]
    headers: Vec<KV>,
}

/// `POST /workspaces/{wid}/api-client/grpc/reflect` — list services/methods via
/// the server's reflection API (no .proto upload).
pub async fn reflect(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<GrpcReflectReq>,
) -> ApiResult<Json<GrpcDescribeResp>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let pool = reflect_pool(&req.url, &req.headers).await?;
    Ok(Json(GrpcDescribeResp { services: services_from_pool(&pool) }))
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

    const SAMPLE: &str = r#"
        syntax = "proto3";
        package demo;
        message HelloRequest { string name = 1; int32 count = 2; }
        message HelloReply { string message = 1; repeated string tags = 3; }
        service Greeter {
          rpc SayHello (HelloRequest) returns (HelloReply);
        }
    "#;

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

    #[test]
    fn json_skeleton_has_fields_with_defaults() {
        let pool = pool_from_proto(SAMPLE).expect("compile proto");
        let req = pool.get_message_by_name("demo.HelloRequest").expect("msg");
        let skeleton: Value = serde_json::from_str(&json_skeleton(&req, 0)).unwrap();
        assert_eq!(skeleton["name"], json!(""));
        assert_eq!(skeleton["count"], json!(0));
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
        assert_eq!(back.get_field_by_name("list_services").and_then(|v| v.as_str().map(String::from)), Some("*".to_string()));
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
}
