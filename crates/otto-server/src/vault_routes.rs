//! Vault v2 — remote backend configuration + auto-install routes.
//!
//! Per-workspace Qdrant (vector) / SurrealDB (graph) / Ollama (embed) backends.
//! Non-secret config lives in the `vault_backends` table; secrets (Qdrant API
//! key, SurrealDB password) go through the Keychain by reference. Enabling a
//! backend registers it on the live `MemoryService`; the daemon re-registers all
//! enabled backends at boot via [`apply_configured_backends`].
//!
//! Paths (relative to `/api/v1`):
//!   GET  /workspaces/{ws}/vault/backends                       (Viewer)
//!   PUT  /workspaces/{ws}/vault/backends/{kind}                (Admin)
//!   POST /workspaces/{ws}/vault/backends/{kind}/health         (Editor)
//!   POST /workspaces/{ws}/vault/backends/{kind}/install/plan   (Editor)
//!   POST /workspaces/{ws}/vault/backends/{kind}/install        (root, DANGEROUS)

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use otto_core::domain::WorkspaceRole;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id};
use serde::{Deserialize, Serialize};

use otto_memory::backends::installer::{self, InstallPlan, InstallResult};
use otto_memory::backends::{QdrantIndex, SurrealClient};
use otto_memory::MemoryService;
use otto_state::{VaultBackend, VaultBackendsRepo};

use crate::auth::{require_root, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

#[derive(Debug, Deserialize)]
pub struct BackendReq {
    pub enabled: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub config_json: Option<String>,
    /// Secret (api key / password); stored in the Keychain, never echoed back.
    #[serde(default)]
    pub secret: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HealthResp {
    pub status: String,
    pub message: Option<String>,
}

fn valid_kind(kind: &str) -> bool {
    matches!(kind, "qdrant" | "surreal" | "ollama")
}

fn secret_key(ws: &str, kind: &str) -> String {
    format!("vault_{kind}_secret_{ws}")
}

fn collection_for(ws: &str) -> String {
    format!("otto_{ws}")
}

/// Build a Qdrant index handle from a backend row + the resolved api key.
fn qdrant_from(b: &VaultBackend, key: Option<String>) -> QdrantIndex {
    QdrantIndex::new(&b.url, key, collection_for(&b.workspace_id))
}

/// Build a SurrealDB client from a backend row's config_json + resolved password.
fn surreal_from(b: &VaultBackend, pass: Option<String>) -> SurrealClient {
    let cfg: serde_json::Value = serde_json::from_str(&b.config_json).unwrap_or_default();
    let ns = cfg.get("ns").and_then(|v| v.as_str()).unwrap_or("otto");
    let db = cfg.get("db").and_then(|v| v.as_str()).unwrap_or("vault");
    let user = cfg.get("user").and_then(|v| v.as_str()).map(|s| s.to_string());
    SurrealClient::new(&b.url, ns, db, user, pass)
}

/// Register one backend on the live memory service (or clear it when disabled).
/// Returns a (status, message) health summary.
async fn register(
    memory: &MemoryService,
    secrets: &Arc<dyn SecretStore>,
    b: &VaultBackend,
) -> (String, Option<String>) {
    let ws = b.workspace_id.clone();
    if !b.enabled {
        match b.kind.as_str() {
            "qdrant" => memory.set_qdrant(&ws, None),
            "surreal" => memory.set_surreal(&ws, None),
            _ => {}
        }
        return ("unknown".into(), Some("disabled".into()));
    }
    let secret = secrets.get(&secret_key(&ws, &b.kind)).ok().flatten();
    match b.kind.as_str() {
        "qdrant" => {
            let q = Arc::new(qdrant_from(b, secret));
            match q.health().await {
                Ok(()) => {
                    memory.set_qdrant(&ws, Some(q));
                    ("ok".into(), None)
                }
                Err(e) => ("error".into(), Some(e.to_string())),
            }
        }
        "surreal" => {
            let s = Arc::new(surreal_from(b, secret));
            match s.health().await {
                Ok(()) => {
                    memory.set_surreal(&ws, Some(s));
                    ("ok".into(), None)
                }
                Err(e) => ("error".into(), Some(e.to_string())),
            }
        }
        "ollama" => {
            // Ollama is the embed layer (global), configured via /memory/embedder;
            // here we only health-check that the server answers.
            let url = if b.url.is_empty() { "http://127.0.0.1:11434".to_string() } else { b.url.clone() };
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_default();
            match client.get(format!("{}/api/tags", url.trim_end_matches('/'))).send().await {
                Ok(r) if r.status().is_success() => ("ok".into(), None),
                Ok(r) => ("error".into(), Some(format!("ollama {}", r.status()))),
                Err(e) => ("error".into(), Some(e.to_string())),
            }
        }
        _ => ("error".into(), Some("unknown kind".into())),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(ws): Path<Id>,
) -> ApiResult<Json<Vec<VaultBackend>>> {
    crate::auth::require_ws_role(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    let repo = VaultBackendsRepo::new(ctx.pool.clone());
    Ok(Json(repo.list(&ws).await.map_err(ApiError)?))
}

async fn upsert(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path((ws, kind)): Path<(Id, String)>,
    Json(req): Json<BackendReq>,
) -> ApiResult<Json<VaultBackend>> {
    crate::auth::require_ws_role(&ctx, &user, &ws, WorkspaceRole::Admin).await?;
    if !valid_kind(&kind) {
        return Err(ApiError(Error::Invalid(format!("unknown backend: {kind}"))));
    }
    // Store secret in the Keychain (never the DB).
    if let Some(sec) = req.secret.as_ref().filter(|s| !s.trim().is_empty()) {
        ctx.secrets.put(&secret_key(&ws, &kind), sec.trim()).map_err(ApiError)?;
    }
    let repo = VaultBackendsRepo::new(ctx.pool.clone());
    let role = if req.role.is_empty() {
        match kind.as_str() {
            "qdrant" => "vector",
            "surreal" => "graph",
            _ => "embed",
        }
        .to_string()
    } else {
        req.role.clone()
    };
    let mut b = repo
        .upsert(&ws, &kind, req.enabled, &req.url, &role, req.config_json.as_deref().unwrap_or("{}"))
        .await
        .map_err(ApiError)?;
    // Register on the live service + reflect health.
    let (status, message) = register(&ctx.memory, &ctx.secrets, &b).await;
    repo.set_status(&ws, &kind, &status, message.as_deref()).await.map_err(ApiError)?;
    b.status = status;
    b.message = message;
    Ok(Json(b))
}

async fn health(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path((ws, kind)): Path<(Id, String)>,
) -> ApiResult<Json<HealthResp>> {
    crate::auth::require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let repo = VaultBackendsRepo::new(ctx.pool.clone());
    let Some(b) = repo.get(&ws, &kind).await.map_err(ApiError)? else {
        return Err(ApiError(Error::NotFound("backend".into())));
    };
    let (status, message) = register(&ctx.memory, &ctx.secrets, &b).await;
    repo.set_status(&ws, &kind, &status, message.as_deref()).await.map_err(ApiError)?;
    Ok(Json(HealthResp { status, message }))
}

async fn install_plan(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path((ws, kind)): Path<(Id, String)>,
) -> ApiResult<Json<InstallPlan>> {
    crate::auth::require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let data_dir = ctx.data_dir.join("vault").to_string_lossy().to_string();
    installer::plan(&kind, &data_dir).await.map(Json).map_err(ApiError)
}

async fn install(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path((ws, kind)): Path<(Id, String)>,
) -> ApiResult<Json<InstallResult>> {
    // Installing changes the host — root only (DANGEROUS).
    require_root(&user)?;
    crate::auth::require_ws_role(&ctx, &user, &ws, WorkspaceRole::Admin).await?;
    let data_dir = ctx.data_dir.join("vault").to_string_lossy().to_string();
    let plan = installer::plan(&kind, &data_dir).await.map_err(ApiError)?;
    if plan.steps.is_empty() {
        return Err(ApiError(Error::Invalid(plan.notes)));
    }
    let repo = VaultBackendsRepo::new(ctx.pool.clone());
    let _ = repo.set_status(&ws, &kind, "installing", None).await;
    let res = installer::execute(&plan).await;
    let status = if res.ok { "ok" } else { "error" };
    let _ = repo.set_status(&ws, &kind, status, Some(&truncate(&res.log, 500))).await;
    Ok(Json(res))
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() > n {
        format!("…{}", &s[s.len() - n..])
    } else {
        s.to_string()
    }
}

/// Re-register all enabled backends on the live memory service at daemon boot.
pub async fn apply_configured_backends(ctx: &ServerCtx) {
    let repo = VaultBackendsRepo::new(ctx.pool.clone());
    let backends = match repo.all_enabled().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("vault: could not load backends: {e}");
            return;
        }
    };
    for b in backends {
        let (status, message) = register(&ctx.memory, &ctx.secrets, &b).await;
        if status == "ok" {
            tracing::info!("vault: backend {} ({}) registered for ws {}", b.kind, b.role, b.workspace_id);
        } else {
            tracing::warn!("vault: backend {} for ws {} not ready: {:?}", b.kind, b.workspace_id, message);
        }
        let _ = repo.set_status(&b.workspace_id, &b.kind, &status, message.as_deref()).await;
    }
}

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{ws}/vault/backends", get(list))
        .route("/workspaces/{ws}/vault/backends/{kind}", put(upsert))
        .route("/workspaces/{ws}/vault/backends/{kind}/health", post(health))
        .route("/workspaces/{ws}/vault/backends/{kind}/install/plan", post(install_plan))
        .route("/workspaces/{ws}/vault/backends/{kind}/install", post(install))
}
