//! The outward "Otto as an MCP server" — **Streamable HTTP transport** glue.
//!
//! The JSON-RPC framing (`initialize` / `tools/list` / `tools/call`, batches,
//! notifications → 202) lives in [`otto_mcp::outward::jsonrpc`]; this module
//! mounts it and implements its narrow [`OutwardTools`] trait over
//! [`ServerCtx`]: `tools/list` is [`mcp_tools_list`] (per-token scope) and every
//! `tools/call` funnels through [`governed_invoke`], the same governed choke
//! point the stdio bridge uses, so enable/approval/audit are identical on both
//! paths.

use async_trait::async_trait;
use axum::extract::State;
use axum::response::Response;
use axum::Json;
use otto_core::auth::AuthContext;
use otto_core::Error;
use otto_mcp::outward::{jsonrpc, OutwardTools};
use serde_json::Value;

use crate::auth::CurrentAuthContext;
use crate::mcp_outward::{governed_invoke, mcp_tools_list};
use crate::state::ServerCtx;

/// [`OutwardTools`] over the server context.
struct ServerTools<'a>(&'a ServerCtx);

#[async_trait]
impl OutwardTools for ServerTools<'_> {
    async fn list(&self, auth: &AuthContext) -> Vec<Value> {
        mcp_tools_list(self.0, auth.mcp_scope.as_ref()).await
    }

    async fn call(&self, auth: &AuthContext, tool: &str, args: &Value) -> Result<Value, Error> {
        governed_invoke(self.0, auth, tool, args, false, None)
            .await
            .map_err(|e| e.0)
    }
}

/// `GET /api/v1/mcp/http` — 405: no standalone SSE stream (see
/// [`jsonrpc::get_response`]).
pub async fn mcp_http_get() -> Response {
    jsonrpc::get_response()
}

/// `POST /api/v1/mcp/http` — one JSON-RPC message or a batch. The caller is
/// already authenticated (`auth_middleware`) and confined (`feature_guard`).
pub async fn mcp_http_post(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(body): Json<Value>,
) -> Response {
    jsonrpc::post_response(&ServerTools(&ctx), &auth, &body, env!("CARGO_PKG_VERSION")).await
}
