//! gRPC / protobuf endpoints for the API client — thin handlers over the
//! `otto_apiclient::grpc` engine (auth + the workspace allow-local opt-in
//! here; parsing, reflection, channels and invocation there):
//!   POST /workspaces/{wid}/api-client/grpc/describe  — parse a `.proto`, list
//!        services/methods + a JSON request skeleton (viewable descriptors).
//!   POST /workspaces/{wid}/api-client/grpc/invoke    — dynamically invoke a
//!        unary method (JSON in → JSON out) through a real gRPC connection.
//!   POST /workspaces/{wid}/api-client/grpc/reflect   — list services/methods
//!        via the server's reflection API (no .proto upload).

use axum::extract::{Path, State};
use axum::Json;

use otto_apiclient::grpc::{
    self, GrpcDescribeReq, GrpcDescribeResp, GrpcInvokeReq, GrpcReflectReq,
};
use otto_core::api::ApiResponse;
use otto_core::domain::WorkspaceRole;
use otto_core::Id;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

pub async fn describe(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<GrpcDescribeReq>,
) -> ApiResult<Json<GrpcDescribeResp>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    Ok(Json(grpc::describe(&req).await?))
}

pub async fn invoke(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<GrpcInvokeReq>,
) -> ApiResult<Json<ApiResponse>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;

    // The workspace's explicit opt-in for local/private targets (grpc against a
    // dev server on localhost is a first-class API-client use case) — honoured
    // by reflection AND invoke alike.
    let allow_local = crate::routes::api_client::workspace_allows_local(&ctx, &wid).await;
    Ok(Json(grpc::invoke(req, allow_local).await?))
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
    let allow_local = crate::routes::api_client::workspace_allows_local(&ctx, &wid).await;
    Ok(Json(grpc::reflect(&req, allow_local).await?))
}
