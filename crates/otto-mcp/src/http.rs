//! MCP Control Plane REST router. Nested under `/api/v1` by `otto-server`, which
//! supplies state via [`McpCtx`]. Feature-axis RBAC (`Feature::Mcp`) is enforced
//! centrally by `otto-server`'s policy guard; these handlers additionally enforce
//! **per-workspace** authorization on flat by-id routes (IDOR, design §14 F13) and
//! the **stdio-server = Admin** rule (RCE-as-daemon, §14 F10).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id};
use otto_state::{
    McpServerDetail, NewAllowlistEntry, NewPolicy, NewServerRow, SqlitePool,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::service::{InvokeCtx, InvokeOutcome, McpService};
use crate::types::*;

/// Server-side context required by the control-plane routes.
pub trait McpCtx: Clone + Send + Sync + 'static {
    fn mcp(&self) -> &Arc<McpService>;
    fn mcp_pool(&self) -> &SqlitePool;
    fn mcp_secrets(&self) -> &Arc<dyn SecretStore>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
}

/// Local problem mapper (orphan rule: can't impl `IntoResponse` for `Error`).
pub struct ApiErr(pub Error);
impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        ApiErr(e)
    }
}
impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(Problem { code: self.0.code().to_string(), message: self.0.to_string() })).into_response()
    }
}
type ApiResult<T> = std::result::Result<T, ApiErr>;

// ---- authz helpers --------------------------------------------------------

async fn require_ws<S: McpCtx>(ctx: &S, user: &User, ws: &Id, min: WorkspaceRole) -> Result<(), ApiErr> {
    ctx.roles().check(user, ws, min).await.map_err(ApiErr)
}

/// Feature availability, workspace membership and resource authority intersect.
async fn require_resource<S: McpCtx>(ctx: &S, user: &User, server: &McpServerDetail, operation: &str, child: Option<&str>, legacy: WorkspaceRole) -> Result<(), ApiErr> {
    let policy = otto_state::ResourceAccessRepo::new(ctx.mcp_pool().clone()).get_policy(otto_core::access::ResourceKind::McpServer, &server.id).await?;
    let min = if policy.mode == otto_core::access::AccessMode::Legacy { legacy } else { WorkspaceRole::Viewer };
    require_ws(ctx,user,&server.workspace_id,min).await?;
    if !ctx.mcp().resource_allowed(server,user,"discover",child).await? { return Err(ApiErr(Error::NotFound("MCP server".into()))); }
    if !ctx.mcp().resource_allowed(server,user,operation,child).await? {
        return Err(ApiErr(Error::Forbidden("MCP resource operation denied".into())));
    }
    Ok(())
}

async fn public_server<S: McpCtx>(ctx: &S, user: &User, mut server: McpServerDetail) -> Result<McpServerDetail,ApiErr> {
    if !ctx.mcp().resource_allowed(&server,user,"configure",None).await? {
        server.command.clear(); server.args.clear(); server.env.clear(); server.url=None; server.headers.clear();
        server.secret_env_keys.clear(); server.secret_header_keys.clear(); server.has_secret=false;
        server.health_error=None;
        server.tools_count=ctx.mcp().visible_tools(&server,user).await?.len() as i64;
    }
    Ok(server)
}

async fn visible_record<S: McpCtx>(ctx: &S, user: &User, server_id: Option<&String>, tool: Option<&str>) -> Result<bool, ApiErr> {
    let Some(id) = server_id else { return Ok(true); };
    let server = match ctx.mcp().registry().get(id).await { Ok(s) => s, Err(Error::NotFound(_)) => return Ok(false), Err(e) => return Err(e.into()) };
    Ok(ctx.mcp().resource_allowed(&server,user,"discover",tool).await?)
}

/// The workspaces a caller may see governance data for. `None` = all (root).
async fn accessible_ws<S: McpCtx>(ctx: &S, user: &User) -> Result<Option<Vec<String>>, ApiErr> {
    if user.is_root {
        return Ok(None);
    }
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT workspace_id FROM workspace_members WHERE user_id = ?",
    )
    .bind(&user.id)
    .fetch_all(ctx.mcp_pool())
    .await
    .map_err(|e| ApiErr(Error::Internal(format!("accessible ws: {e}"))))?;
    Ok(Some(rows))
}

// ===========================================================================
// Router
// ===========================================================================

pub fn api_router<S: McpCtx>() -> Router<S> {
    Router::new()
        // registry (workspace-scoped collection)
        .route("/workspaces/{wid}/mcp/servers", get(list_servers::<S>).post(create_server::<S>))
        .route("/mcp/servers/{id}", get(get_server::<S>).patch(update_server::<S>).delete(delete_server::<S>))
        .route("/mcp/servers/{id}/health", post(health_server::<S>))
        .route("/mcp/servers/{id}/discover", post(discover_server::<S>))
        .route("/mcp/servers/{id}/tools", get(list_tools::<S>))
        .route("/mcp/servers/{id}/tools/{name}/invoke", post(invoke_tool::<S>))
        .route("/mcp/tools/{tool_id}", patch(patch_tool::<S>))
        // allowlists
        .route("/workspaces/{wid}/mcp/allowlist", get(get_allowlist::<S>).put(set_allowlist::<S>))
        // policy-as-code
        .route("/mcp/policies", get(list_policies::<S>).post(create_policy::<S>))
        .route("/mcp/policies/{id}", patch(update_policy::<S>).delete(delete_policy::<S>))
        .route("/mcp/policies/export", get(export_policies::<S>))
        .route("/mcp/policies/import", post(import_policies::<S>))
        .route("/mcp/policies/evaluate", post(evaluate_policy::<S>))
        // approvals
        .route("/mcp/approvals", get(list_approvals::<S>))
        .route("/mcp/approvals/{id}/decide", post(decide_approval::<S>))
        // audit + stats
        .route("/mcp/audit", get(list_audit::<S>))
        .route("/mcp/stats", get(stats::<S>))
}

// ===========================================================================
// Registry handlers
// ===========================================================================

async fn list_servers<S: McpCtx>(
    Path(wid): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<Vec<McpServerDetail>>> {
    require_ws(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let mut visible = Vec::new();
    for server in ctx.mcp().registry().list_for_ws(&wid).await? {
        if ctx.mcp().resource_allowed(&server,&user,"discover",None).await? { visible.push(public_server(&ctx,&user,server).await?); }
    }
    Ok(Json(visible))
}

async fn create_server<S: McpCtx>(
    Path(wid): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<CreateServerReq>,
) -> ApiResult<Json<McpServerDetail>> {
    require_ws(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let transport = req.transport.clone();
    if transport != "stdio" && transport != "http" {
        return Err(ApiErr(Error::Invalid("transport must be 'stdio' or 'http'".into())));
    }
    if !user.is_root { return Err(ApiErr(Error::Forbidden("only the owner can attach MCP credentials or commands".into()))); }
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiErr(Error::Invalid("server name must not be empty".into())));
    }
    if transport == "stdio" && req.command.trim().is_empty() {
        return Err(ApiErr(Error::Invalid("stdio server requires a command".into())));
    }
    if transport == "http" && req.url.as_deref().unwrap_or("").trim().is_empty() {
        return Err(ApiErr(Error::Invalid("http server requires a url".into())));
    }
    let secret_env_keys: Vec<String> = req.secret_env.keys().cloned().collect();
    let secret_header_keys: Vec<String> = req.secret_headers.keys().cloned().collect();
    let server = ctx
        .mcp()
        .registry()
        .create(NewServerRow {
            workspace_id: wid,
            name: name.to_string(),
            transport,
            command: req.command.trim().to_string(),
            args: req.args,
            env: req.env,
            url: req.url,
            description: req.description,
            headers: req.headers,
            secret_ref: None,
            secret_env_keys: secret_env_keys.clone(),
            secret_header_keys: secret_header_keys.clone(),
            injection_risk: req.injection_risk,
            default_tool_access: req.default_tool_access,
            enabled: req.enabled,
            created_by: user.id.clone(),
        })
        .await
        .map_err(ApiErr)?;
    // Persist secret values to the keychain (never to the row).
    if !req.secret_env.is_empty() || !req.secret_headers.is_empty() {
        let blob = json!({ "env": req.secret_env, "headers": req.secret_headers }).to_string();
        let sref = McpService::secret_ref(&server.id);
        ctx.mcp_secrets().put(&sref, &blob).map_err(ApiErr)?;
        ctx.mcp().registry()
            .set_secret_meta(&server.id, Some(&sref), &secret_env_keys, &secret_header_keys)
            .await
            .map_err(ApiErr)?;
    }
    let operations: Vec<String> = otto_core::access::operations_for(otto_core::access::ResourceKind::McpServer).iter().map(|s| s.to_string()).collect();
    otto_state::ResourceAccessRepo::new(ctx.mcp_pool().clone()).initialize_owner_policy(
        otto_core::access::ResourceKind::McpServer, &server.id, &user.id, &operations, &[],
        &otto_core::access::AccessActor { real_user_id:user.id.clone(),effective_user_id:None }).await?;
    ctx.mcp().registry().get(&server.id).await.map(Json).map_err(ApiErr)
}

async fn get_server<S: McpCtx>(
    Path(id): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<Value>> {
    let server = ctx.mcp().registry().get(&id).await.map_err(ApiErr)?;
    require_resource(&ctx, &user, &server, "discover", None, WorkspaceRole::Viewer).await?;
    let tools = ctx.mcp().visible_tools(&server,&user).await.map_err(ApiErr)?;
    Ok(Json(json!({ "server": public_server(&ctx,&user,server).await?, "tools": tools })))
}

async fn update_server<S: McpCtx>(
    Path(id): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<UpdateServerReq>,
) -> ApiResult<Json<McpServerDetail>> {
    let server = ctx.mcp().registry().get(&id).await.map_err(ApiErr)?;
    require_resource(&ctx, &user, &server, "configure", None, WorkspaceRole::Editor).await?;
    if !user.is_root && (req.command.as_ref().is_some_and(|v| v != &server.command)
        || req.args.as_ref().is_some_and(|v| v != &server.args)
        || req.env.as_ref().is_some_and(|v| v != &server.env)
        || req.url.as_ref().is_some_and(|v| Some(v) != server.url.as_ref())
        || req.headers.as_ref().is_some_and(|v| v != &server.headers)
        || req.secret_env.is_some() || req.secret_headers.is_some()) {
        return Err(ApiErr(Error::Forbidden("only the owner can change MCP credentials or commands".into())));
    }
    // Secret rotation: merge new secret values into the keychain blob.
    if req.secret_env.is_some() || req.secret_headers.is_some() {
        let env = req.secret_env.clone().unwrap_or_default();
        let headers = req.secret_headers.clone().unwrap_or_default();
        let blob = json!({ "env": env, "headers": headers }).to_string();
        let sref = McpService::secret_ref(&id);
        ctx.mcp_secrets().put(&sref, &blob).map_err(ApiErr)?;
        let ek: Vec<String> = env.keys().cloned().collect();
        let hk: Vec<String> = headers.keys().cloned().collect();
        ctx.mcp().registry().set_secret_meta(&id, Some(&sref), &ek, &hk).await.map_err(ApiErr)?;
    }
    ctx.mcp()
        .registry()
        .update(
            &id,
            req.name.as_deref(),
            req.description.as_deref(),
            req.command.as_deref(),
            req.args.as_deref(),
            req.env.as_ref(),
            req.url.as_deref(),
            req.headers.as_ref(),
            req.injection_risk.as_deref(),
            req.default_tool_access.as_deref(),
            req.enabled,
        )
        .await
        .map(Json)
        .map_err(ApiErr)
}

async fn delete_server<S: McpCtx>(
    Path(id): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<StatusCode> {
    let server = ctx.mcp().registry().get(&id).await.map_err(ApiErr)?;
    require_resource(&ctx, &user, &server, "configure", None, WorkspaceRole::Editor).await?;
    // Best-effort secret cleanup.
    if server.has_secret {
        let _ = ctx.mcp_secrets().delete(&McpService::secret_ref(&id));
    }
    ctx.mcp().registry().delete(&id).await.map_err(ApiErr)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn health_server<S: McpCtx>(
    Path(id): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<McpServerDetail>> {
    let server = ctx.mcp().registry().get(&id).await.map_err(ApiErr)?;
    require_resource(&ctx, &user, &server, "configure", None, WorkspaceRole::Editor).await?;
    ctx.mcp().health_check(&id).await.map(Json).map_err(ApiErr)
}

async fn discover_server<S: McpCtx>(
    Path(id): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<Vec<otto_state::McpTool>>> {
    let server = ctx.mcp().registry().get(&id).await.map_err(ApiErr)?;
    require_resource(&ctx, &user, &server, "configure", None, WorkspaceRole::Editor).await?;
    ctx.mcp().discover(&id).await.map(Json).map_err(ApiErr)
}

async fn list_tools<S: McpCtx>(
    Path(id): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<Vec<otto_state::McpTool>>> {
    let server = ctx.mcp().registry().get(&id).await.map_err(ApiErr)?;
    require_resource(&ctx, &user, &server, "discover", None, WorkspaceRole::Viewer).await?;
    ctx.mcp().visible_tools(&server,&user).await.map(Json).map_err(ApiErr)
}

async fn patch_tool<S: McpCtx>(
    Path(tool_id): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<PatchToolReq>,
) -> ApiResult<Json<otto_state::McpTool>> {
    let tool = ctx.mcp().tools().get(&tool_id).await.map_err(ApiErr)?;
    let server = ctx.mcp().registry().get(&tool.server_id).await.map_err(ApiErr)?;
    require_resource(&ctx, &user, &server, "configure", Some(&tool.name), WorkspaceRole::Editor).await?;
    ctx.mcp()
        .tools()
        .patch(&tool_id, req.enabled, req.require_approval, req.risk_label.as_deref(), req.injection_risk.as_deref())
        .await
        .map(Json)
        .map_err(ApiErr)
}

async fn invoke_tool<S: McpCtx>(
    Path((id, name)): Path<(Id, String)>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<InvokeReq>,
) -> ApiResult<Json<InvokeResp>> {
    let server = ctx.mcp().registry().get(&id).await.map_err(ApiErr)?;
    require_resource(&ctx, &user, &server, "invoke", Some(&name), WorkspaceRole::Editor).await?;
    let ictx = InvokeCtx {
        workspace_id: Some(server.workspace_id.clone()),
        dry_run: req.dry_run,
        caller_user_id: Some(user.id.clone()),
        caller_kind: "ui".into(),
        direction: "outbound".into(),
    };
    let outcome = ctx.mcp().invoke(&id, &name, &req.arguments, &ictx).await.map_err(ApiErr)?;
    Ok(Json(outcome_to_resp(outcome)))
}

pub fn outcome_to_resp(outcome: InvokeOutcome) -> InvokeResp {
    match outcome {
        InvokeOutcome::Denied { reason } => InvokeResp {
            decision: "denied".into(), executed: false, dry_run: false,
            reason: Some(reason), approval_id: None, content: None, is_error: None, preview: None,
        },
        InvokeOutcome::Pending { approval_id, title } => InvokeResp {
            decision: "pending_approval".into(), executed: false, dry_run: false,
            reason: Some(format!("awaiting human approval: {title}")),
            approval_id: Some(approval_id), content: None, is_error: None, preview: None,
        },
        InvokeOutcome::DryRun { preview } => InvokeResp {
            decision: "dry_run".into(), executed: false, dry_run: true,
            reason: None, approval_id: None, content: None, is_error: None, preview: Some(preview),
        },
        InvokeOutcome::Executed { content, is_error } => InvokeResp {
            decision: "allowed".into(), executed: true, dry_run: false,
            reason: None, approval_id: None, content: Some(content), is_error: Some(is_error), preview: None,
        },
    }
}

// ===========================================================================
// Allowlist handlers
// ===========================================================================

async fn get_allowlist<S: McpCtx>(
    Path(wid): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<Vec<otto_state::McpAllowlistEntry>>> {
    require_ws(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let mut visible = Vec::new();
    for row in ctx.mcp().allowlist().list_for_ws(&wid).await? {
        if visible_record(&ctx,&user,Some(&row.server_id),row.tool_name.as_deref()).await? { visible.push(row); }
    }
    Ok(Json(visible))
}

async fn set_allowlist<S: McpCtx>(
    Path(wid): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<SetAllowlistReq>,
) -> ApiResult<StatusCode> {
    require_ws(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    // Every referenced server must belong to THIS workspace (no cross-ws allowlist).
    for e in &req.entries {
        let s = ctx.mcp().registry().get(&e.server_id).await.map_err(ApiErr)?;
        if s.workspace_id != wid {
            return Err(ApiErr(Error::Invalid(format!(
                "server {} is not in this workspace",
                e.server_id
            ))));
        }
        if e.mode != "allow" && e.mode != "deny" {
            return Err(ApiErr(Error::Invalid("mode must be 'allow' or 'deny'".into())));
        }
    }
    let entries: Vec<NewAllowlistEntry> = req
        .entries
        .into_iter()
        .map(|e| NewAllowlistEntry { server_id: e.server_id, tool_name: e.tool_name, mode: e.mode })
        .collect();
    ctx.mcp().allowlist().replace_for_ws(&wid, &entries, &user.id).await.map_err(ApiErr)?;
    Ok(StatusCode::NO_CONTENT)
}

// ===========================================================================
// Policy handlers
// ===========================================================================

#[derive(Deserialize)]
struct PolicyListQuery {
    workspace_id: Option<String>,
}

async fn list_policies<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Query(q): Query<PolicyListQuery>,
) -> ApiResult<Json<Vec<otto_state::McpPolicy>>> {
    ctx.mcp().policies().list(q.workspace_id.as_ref()).await.map(Json).map_err(ApiErr)
}

async fn create_policy<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<CreatePolicyReq>,
) -> ApiResult<Json<otto_state::McpPolicy>> {
    validate_effect(&req.effect)?;
    ctx.mcp()
        .policies()
        .create(NewPolicy {
            workspace_id: req.workspace_id,
            name: req.name,
            enabled: req.enabled,
            priority: req.priority,
            match_json: req.match_json,
            effect: req.effect,
            reason: req.reason,
            created_by: user.id,
        })
        .await
        .map(Json)
        .map_err(ApiErr)
}

async fn update_policy<S: McpCtx>(
    Path(id): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Json(req): Json<UpdatePolicyReq>,
) -> ApiResult<Json<otto_state::McpPolicy>> {
    if let Some(e) = &req.effect {
        validate_effect(e)?;
    }
    ctx.mcp()
        .policies()
        .update(&id, req.name.as_deref(), req.enabled, req.priority, req.match_json.as_ref(), req.effect.as_deref(), req.reason.as_deref())
        .await
        .map(Json)
        .map_err(ApiErr)
}

async fn delete_policy<S: McpCtx>(
    Path(id): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<StatusCode> {
    ctx.mcp().policies().delete(&id).await.map_err(ApiErr)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn export_policies<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<Json<Value>> {
    let all = ctx.mcp().policies().list(None).await.map_err(ApiErr)?;
    Ok(Json(json!({ "version": 1, "policies": all })))
}

async fn import_policies<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<ImportPoliciesReq>,
) -> ApiResult<Json<Value>> {
    for p in &req.policies {
        validate_effect(&p.effect)?;
    }
    if req.replace {
        // Clear existing rules then re-create. (Admin-gated route.)
        let existing = ctx.mcp().policies().list(None).await.map_err(ApiErr)?;
        for p in existing {
            ctx.mcp().policies().delete(&p.id).await.map_err(ApiErr)?;
        }
    }
    let mut created = 0;
    for p in req.policies {
        ctx.mcp()
            .policies()
            .create(NewPolicy {
                workspace_id: p.workspace_id,
                name: p.name,
                enabled: p.enabled,
                priority: p.priority,
                match_json: p.match_json,
                effect: p.effect,
                reason: p.reason,
                created_by: user.id.clone(),
            })
            .await
            .map_err(ApiErr)?;
        created += 1;
    }
    Ok(Json(json!({ "imported": created, "replaced": req.replace })))
}

async fn evaluate_policy<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<EvaluateReq>,
) -> ApiResult<Json<Value>> {
    // Read-only preview; still confirm the caller can see the server's workspace.
    let server = ctx.mcp().registry().get(&req.server_id).await.map_err(ApiErr)?;
    require_ws(&ctx, &user, &server.workspace_id, WorkspaceRole::Viewer).await?;
    ctx.mcp()
        .evaluate_preview(&req.server_id, &req.tool, req.workspace_id.as_deref())
        .await
        .map(Json)
        .map_err(ApiErr)
}

fn validate_effect(effect: &str) -> Result<(), ApiErr> {
    match effect {
        "allow" | "deny" | "require_approval" | "require_dry_run" => Ok(()),
        _ => Err(ApiErr(Error::Invalid(
            "effect must be allow|deny|require_approval|require_dry_run".into(),
        ))),
    }
}

// ===========================================================================
// Approvals / audit / stats
// ===========================================================================

#[derive(Deserialize)]
struct ApprovalQuery {
    status: Option<String>,
}

async fn list_approvals<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<ApprovalQuery>,
) -> ApiResult<Json<Vec<otto_state::McpApproval>>> {
    let ws = accessible_ws(&ctx, &user).await?;
    let mut visible = Vec::new();
    for row in ctx.mcp().approvals().list(ws.as_deref(),q.status.as_deref(),200).await? {
        if visible_record(&ctx,&user,row.server_id.as_ref(),row.tool.as_deref()).await? { visible.push(row); }
    }
    Ok(Json(visible))
}

async fn decide_approval<S: McpCtx>(
    Path(id): Path<Id>,
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    auth: Option<Extension<otto_core::auth::AuthContext>>,
    Json(req): Json<DecideReq>,
) -> ApiResult<Json<otto_state::McpApproval>> {
    let appr = ctx.mcp().approvals().get(&id).await.map_err(ApiErr)?;
    if auth.as_ref().is_some_and(|a| appr.requested_by.as_ref() == Some(&a.0.real_user.id)) {
        return Err(Error::Forbidden("requester cannot approve through impersonation".into()).into());
    }
    // IDOR: the decider must have a role in the approval's workspace.
    if appr.server_id.is_none() {
        if let Some(ws) = &appr.workspace_id { require_ws(&ctx,&user,ws,WorkspaceRole::Editor).await?; }
    }
    if let Some(server_id) = &appr.server_id {
        let server = ctx.mcp().registry().get(server_id).await?;
        require_resource(&ctx,&user,&server,"approve",appr.tool.as_deref(),WorkspaceRole::Editor).await?;
    }
    // Repo enforces approver != requester (separation of duties).
    ctx.mcp()
        .approvals()
        .decide(&id, req.approved, &user.id, req.note.as_deref())
        .await
        .map(Json)
        .map_err(ApiErr)
}

#[derive(Deserialize)]
struct AuditQuery {
    server_id: Option<String>,
    tool: Option<String>,
    decision: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list_audit<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Vec<otto_state::McpCallLogRow>>> {
    let ws = accessible_ws(&ctx, &user).await?;
    let query = otto_state::CallLogQuery {
        workspace_ids: ws,
        server_id: q.server_id,
        tool: q.tool,
        decision: q.decision,
        limit: q.limit.unwrap_or(200),
        offset: q.offset.unwrap_or(0),
    };
    let mut visible = Vec::new();
    for row in ctx.mcp().call_log().list(&query).await? {
        if visible_record(&ctx,&user,row.server_id.as_ref(),Some(&row.tool)).await? { visible.push(row); }
    }
    Ok(Json(visible))
}

async fn stats<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<Vec<otto_state::McpToolStats>>> {
    let ws = accessible_ws(&ctx, &user).await?;
    let mut visible = Vec::new();
    for row in ctx.mcp().call_log().stats(ws.as_deref()).await? {
        if visible_record(&ctx,&user,row.server_id.as_ref(),Some(&row.tool)).await? { visible.push(row); }
    }
    Ok(Json(visible))
}
