//! API-client engines split out of `otto-server` (compile-time isolation of
//! the heavy tonic / protox / prost-reflect / boa_engine dependency trees),
//! plus the pure REST pieces (`rest`: curl parsing, OpenAPI export, JSON
//! path, assertions), the REST execute engine (`engine`, behind
//! [`engine::ApiClientCtx`]) and the Keychain-backed request / environment
//! secrets (`secrets`).
//! Engine code only: no web framework, no server context type — the
//! server's thin route handlers do auth and workspace lookups, then call in
//! here.

pub mod engine;
pub mod grpc;
pub mod rest;
pub mod scripts;
pub mod secrets;
