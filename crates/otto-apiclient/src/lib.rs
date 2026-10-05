//! API-client engines split out of `otto-server` (compile-time isolation of
//! the heavy tonic / protox / prost-reflect / boa_engine dependency trees).
//! Pure engine code: no axum, no `ServerCtx` — the server's thin route
//! handlers do auth and workspace lookups, then call in here.

pub mod grpc;
pub mod scripts;
