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

use crate::auth::{require_ws_role, CurrentAuthContext, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;
use axum::http::HeaderMap;

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
    CurrentAuthContext(auth): CurrentAuthContext,
    headers: HeaderMap,
    Json(mut req): Json<WorkspaceGrpcInvoke>,
) -> ApiResult<Json<ApiResponse>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;

    // The workspace's explicit opt-in for local/private targets (grpc against a
    // dev server on localhost is a first-class API-client use case) — honoured
    // by reflection AND invoke alike.
    let allow_local = crate::routes::api_client::workspace_allows_local(&ctx, &wid).await;
    let agent = super::api_client::is_agent_caller(&headers, &auth);
    let secrets = resolve_request(
        &ctx,
        &wid,
        &mut req.request.url,
        &mut req.request.headers,
        Some(&mut req.request.body),
        &req.options,
        agent,
    )
    .await?;
    let mut response = grpc::invoke(req.request, allow_local)
        .await
        .map_err(|e| scrub_error(e, &secrets))?;
    if agent {
        crate::api_secrets::scrub_secrets(&mut response, &secrets);
    }
    Ok(Json(response))
}

/// `POST /workspaces/{wid}/api-client/grpc/reflect` — list services/methods via
/// the server's reflection API (no .proto upload).
pub async fn reflect(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    headers: HeaderMap,
    Json(mut req): Json<WorkspaceGrpcReflect>,
) -> ApiResult<Json<GrpcDescribeResp>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let allow_local = crate::routes::api_client::workspace_allows_local(&ctx, &wid).await;
    let agent = super::api_client::is_agent_caller(&headers, &auth);
    let secrets = resolve_request(
        &ctx,
        &wid,
        &mut req.request.url,
        &mut req.request.headers,
        None,
        &req.options,
        agent,
    )
    .await?;
    Ok(Json(
        grpc::reflect(&req.request, allow_local)
            .await
            .map_err(|e| scrub_error(e, &secrets))?,
    ))
}

#[derive(serde::Deserialize, Default)]
pub struct GrpcOptions {
    #[serde(default)]
    environment_id: Option<Id>,
    #[serde(default)]
    vars: Option<serde_json::Value>,
    #[serde(default)]
    confirm_new_host: bool,
}

#[derive(serde::Deserialize)]
pub struct WorkspaceGrpcInvoke {
    #[serde(flatten)]
    request: GrpcInvokeReq,
    #[serde(flatten)]
    options: GrpcOptions,
}

#[derive(serde::Deserialize)]
pub struct WorkspaceGrpcReflect {
    #[serde(flatten)]
    request: GrpcReflectReq,
    #[serde(flatten)]
    options: GrpcOptions,
}

/// Resolve JSON string values after parsing, so quotes/newlines in environment
/// values cannot corrupt the message container. Protobuf still validates types.
fn substitute_message(
    body: &str,
    vars: &serde_json::Map<String, serde_json::Value>,
) -> Result<String, otto_core::Error> {
    fn visit(value: &mut serde_json::Value, vars: &serde_json::Map<String, serde_json::Value>) {
        match value {
            serde_json::Value::String(text) => *text = super::api_client::substitute(text, vars),
            serde_json::Value::Array(values) => values.iter_mut().for_each(|v| visit(v, vars)),
            serde_json::Value::Object(values) => values.values_mut().for_each(|v| visit(v, vars)),
            _ => {}
        }
    }
    let mut value: serde_json::Value =
        serde_json::from_str(if body.trim().is_empty() { "{}" } else { body })
            .map_err(|e| otto_core::Error::Invalid(format!("Invalid gRPC message JSON: {e}")))?;
    visit(&mut value, vars);
    Ok(value.to_string())
}

#[allow(clippy::too_many_arguments)]
async fn resolve_request(
    ctx: &ServerCtx,
    wid: &Id,
    url: &mut String,
    metadata: &mut [grpc::KV],
    body: Option<&mut String>,
    options: &GrpcOptions,
    agent: bool,
) -> ApiResult<Vec<String>> {
    use super::api_client::{
        new_host_conflict, resolve_environment, substitute, unbound_secret_host,
    };
    use serde_json::json;
    let repo = otto_state::ApiClientRepo::new(ctx.pool.clone());
    let (mut vars, env, blob) =
        resolve_environment(ctx, &repo, wid, options.environment_id.as_ref()).await?;
    let bind_vars = vars.clone();
    if let Some(overrides) = &options.vars {
        let overrides = overrides.as_object().ok_or_else(|| {
            crate::error::ApiError(otto_core::Error::Invalid(
                "vars must be a JSON object".into(),
            ))
        })?;
        vars.extend(overrides.clone());
    }
    // Use the existing HTTP binding policy on the unresolved transport input.
    let probe: otto_core::api::ExecuteApiReq = serde_json::from_value(json!({
        "method":"POST", "url":url, "headers":metadata, "body":body.as_deref().map(String::as_str).unwrap_or(""),
        "body_mode":"json", "auth":{"type":"none"}
    })).expect("valid execution probe");
    let keys = env
        .as_ref()
        .map(|e| e.secret_keys.as_slice())
        .unwrap_or(&[]);
    if let Some(host) =
        unbound_secret_host(&repo, wid, &probe, &vars, &bind_vars, &blob, keys).await?
    {
        if agent || !options.confirm_new_host {
            return Err(new_host_conflict(&host, agent));
        }
    }
    let secrets: Vec<String> = blob.values().cloned().collect();
    // Validate/resolve the body before mutating the URL or metadata.
    if let Some(body) = body {
        *body = substitute_message(body, &vars)?;
    }
    *url = substitute(url, &vars);
    for header in metadata {
        header.key = substitute(&header.key, &vars);
        header.value = substitute(&header.value, &vars);
    }
    Ok(secrets)
}

fn scrub_error(error: otto_core::Error, secrets: &[String]) -> crate::error::ApiError {
    use otto_core::Error;
    crate::error::ApiError(match error {
        Error::Invalid(message) => Error::Invalid(crate::api_secrets::scrub_str(&message, secrets)),
        Error::Upstream(message) => {
            Error::Upstream(crate::api_secrets::scrub_str(&message, secrets))
        }
        error => error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn message_variables_preserve_json_escaping_and_nested_types() {
        let vars = json!({"tenant":"a\"b\n\\&"}).as_object().unwrap().clone();
        let body = substitute_message(
            r#"{"tenant":"{{tenant}}","nested":["{{tenant}}",true,3]}"#,
            &vars,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap(),
            json!({"tenant":"a\"b\n\\&","nested":["a\"b\n\\&",true,3]})
        );
        assert!(substitute_message("{broken", &vars).is_err());
        assert_eq!(substitute_message("", &vars).unwrap(), "{}");
    }

    #[test]
    fn wrapper_accepts_legacy_requests_and_workspace_resolution_options() {
        let old: WorkspaceGrpcInvoke =
            serde_json::from_value(json!({"url":"https://example.test","method":"/Test/Read"}))
                .unwrap();
        assert!(old.options.environment_id.is_none());
        let new: WorkspaceGrpcReflect = serde_json::from_value(json!({"url":"{{host}}","headers":[{"key":"authorization","value":"{{token}}"}],"environment_id":"env","vars":{"host":"https://example.test"},"confirm_new_host":true})).unwrap();
        assert_eq!(new.options.environment_id.as_deref(), Some("env"));
        assert!(new.options.confirm_new_host);
        assert_eq!(new.request.headers[0].value, "{{token}}");
    }
    #[tokio::test]
    async fn grpc_resolves_workspace_environment_and_enforces_secret_host_binding() {
        use crate::routes::browser::tests::{mem_pool, root_user, seed_workspace, test_ctx};
        let pool = mem_pool().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let wid: Id = "grpc-env-test".into();
        seed_workspace(&pool, &wid).await;
        let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
        let Json(env) = super::super::api_client::create_environment(Path(wid.clone()), State(ctx.clone()), CurrentUser(root_user()),
            Json(serde_json::from_value(json!({"name":"dev","variables":{"host":"http://127.0.0.1:50051","tenant":"plain"},"secret_keys":["token"],"secret_values":{"token":"stored-token"}})).unwrap())).await.unwrap();
        let options: GrpcOptions =
            serde_json::from_value(json!({"environment_id":env.id,"vars":{"tenant":"override"}}))
                .unwrap();
        let mut url = "{{host}}".to_string();
        let mut headers: Vec<grpc::KV> =
            serde_json::from_value(json!([{"key":"x-tenant","value":"{{tenant}}"}])).unwrap();
        let mut body = r#"{"tenant":"{{tenant}}"}"#.to_string();
        resolve_request(
            &ctx,
            &wid,
            &mut url,
            &mut headers,
            Some(&mut body),
            &options,
            false,
        )
        .await
        .unwrap();
        assert_eq!(url, "http://127.0.0.1:50051");
        assert_eq!(headers[0].value, "override");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap(),
            json!({"tenant":"override"})
        );
        let mut headers: Vec<grpc::KV> =
            serde_json::from_value(json!([{"key":"authorization","value":"Bearer {{token}}"}]))
                .unwrap();
        let error = resolve_request(&ctx, &wid, &mut url, &mut headers, None, &options, false)
            .await
            .unwrap_err();
        assert!(error.0.to_string().contains("needs_confirm=new_host"));
        let confirmed = GrpcOptions {
            confirm_new_host: true,
            ..options
        };
        assert!(
            resolve_request(&ctx, &wid, &mut url, &mut headers, None, &confirmed, true)
                .await
                .is_err()
        );
        resolve_request(&ctx, &wid, &mut url, &mut headers, None, &confirmed, false)
            .await
            .unwrap();
        assert_eq!(headers[0].value, "Bearer stored-token");
        let foreign: Id = "grpc-other-workspace".into();
        seed_workspace(&pool, &foreign).await;
        assert!(resolve_request(
            &ctx,
            &foreign,
            &mut url,
            &mut headers,
            None,
            &confirmed,
            false
        )
        .await
        .is_err());
    }
}
