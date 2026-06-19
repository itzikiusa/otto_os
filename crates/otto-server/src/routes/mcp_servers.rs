//! Workspace MCP-server config CRUD. Users manage the MCP servers that get
//! merged into a workspace's `.mcp.json` when an agent session spawns there
//! (alongside Otto's own managed browser entry — see `otto-sessions::mcp`).
//!
//! Nothing here is auto-enabled: `enabled` defaults to `false` on create, and a
//! server is only written to `.mcp.json` once the user flips it on and a session
//! then spawns. Reads = `ws viewer`, mutations = `ws editor`. Item routes
//! resolve the owning workspace from the row.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::api::{CreateMcpServerReq, UpdateMcpServerReq};
use otto_core::domain::{McpServer, WorkspaceRole};
use otto_core::hooks::{McpServerProvider, McpServerSpec};
use otto_core::{Error, Id};
use otto_state::{McpServersRepo, NewMcpServer};

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

fn repo(ctx: &ServerCtx) -> McpServersRepo {
    McpServersRepo::new(ctx.pool.clone())
}

/// `GET /api/v1/workspaces/{id}/mcp-servers` — ws viewer.
pub async fn list(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<McpServer>>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list_for_ws(&ws_id).await?))
}

/// `POST /api/v1/workspaces/{id}/mcp-servers` — ws editor. `enabled` defaults
/// off; a server is never auto-enabled.
pub async fn create(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateMcpServerReq>,
) -> ApiResult<Json<McpServer>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("mcp server name must not be empty".into()).into());
    }
    if req.command.trim().is_empty() {
        return Err(Error::Invalid("mcp server command must not be empty".into()).into());
    }
    let server = repo(&ctx)
        .create(NewMcpServer {
            workspace_id: ws_id,
            name: name.to_string(),
            command: req.command.trim().to_string(),
            args: req.args,
            env: req.env,
            enabled: req.enabled,
            created_by: user.id,
        })
        .await?;
    Ok(Json(server))
}

/// `PATCH /api/v1/mcp-servers/{id}` — ws editor (workspace resolved from the row).
pub async fn update(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpdateMcpServerReq>,
) -> ApiResult<Json<McpServer>> {
    let repo = repo(&ctx);
    let existing = repo.get(&id).await?;
    require_ws_role(&ctx, &user, &existing.workspace_id, WorkspaceRole::Editor).await?;
    let name = match req.name.as_deref().map(str::trim) {
        Some("") => return Err(Error::Invalid("mcp server name must not be empty".into()).into()),
        other => other,
    };
    let command = match req.command.as_deref().map(str::trim) {
        Some("") => {
            return Err(Error::Invalid("mcp server command must not be empty".into()).into())
        }
        other => other,
    };
    let server = repo
        .update(
            &id,
            name,
            command,
            req.args.as_deref(),
            req.env.as_ref(),
            req.enabled,
        )
        .await?;
    Ok(Json(server))
}

/// `DELETE /api/v1/mcp-servers/{id}` — ws editor (workspace resolved from the row).
pub async fn delete(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let repo = repo(&ctx);
    let existing = repo.get(&id).await?;
    require_ws_role(&ctx, &user, &existing.workspace_id, WorkspaceRole::Editor).await?;
    repo.delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `McpServerProvider` backed by the SQLite repo: resolves a workspace's enabled
/// servers for the session manager to merge into `.mcp.json` at spawn. Sync trait
/// over an async repo, so it blocks briefly on the current Tokio runtime — fine
/// for the handful of rows per workspace, and best-effort (errors → empty).
#[derive(Clone)]
pub struct DbMcpServerProvider {
    pool: otto_state::SqlitePool,
}

impl DbMcpServerProvider {
    pub fn new(pool: otto_state::SqlitePool) -> Self {
        Self { pool }
    }
}

impl McpServerProvider for DbMcpServerProvider {
    fn enabled_servers(&self, workspace_id: &str) -> Vec<McpServerSpec> {
        let repo = McpServersRepo::new(self.pool.clone());
        let ws = workspace_id.to_string();
        // Bridge the async repo onto the calling thread without holding the
        // runtime: spawn the query and block this thread on its result.
        let servers = std::thread::scope(|s| {
            s.spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .ok()
                    .and_then(|rt| rt.block_on(repo.list_enabled(&ws)).ok())
            })
            .join()
            .ok()
            .flatten()
        });
        match servers {
            Some(rows) => rows
                .into_iter()
                .map(|r| McpServerSpec {
                    name: r.name,
                    command: r.command,
                    args: r.args,
                    env: r.env,
                })
                .collect(),
            None => Vec::new(),
        }
    }
}
