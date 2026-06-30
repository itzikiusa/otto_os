//! Vault embedder configuration: wire a real OpenAI/Voyage embedder (or the
//! deterministic local stub) into the running memory service.
//!
//! Paths (relative to `/api/v1`):
//!   GET  /memory/embedder                    — active embedder status (View)
//!   PUT  /memory/embedder                    — switch provider + store key (Admin)
//!   POST /workspaces/{ws}/memory/reindex     — re-embed a workspace (Edit)
//!
//! The provider key is stored in the keychain (`<provider>_api_key`), never the
//! settings DB. The embedder swaps in live (the memory service holds it behind a
//! lock); a `dim` change is reconciled by `reindex`. The daemon applies the
//! persisted setting at boot via [`apply_configured_embedder`].

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use otto_core::domain::WorkspaceRole;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};

use otto_memory::embed::{
    Embedder, LocalCodeEmbedder, OllamaEmbedder, RemoteEmbedder, RemoteProvider, StubEmbedder,
};
use otto_memory::MemoryService;
use otto_state::SettingsRepo;

use crate::auth::{require_root, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// Settings key holding the embedder choice (`{provider, ...}`). The key lives
/// in the Keychain, not here.
pub const EMBEDDER_KEY: &str = "embedder";
/// Dimension of the built-in deterministic stub embedder (matches the memory
/// service default).
const STUB_DIM: usize = 256;

/// Persisted embedder config (the API key is NOT stored here — it is a Keychain
/// reference resolved at build time).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedderConfig {
    /// `"local"` (default, code-aware) | `"ollama"` | `"openai"` | `"voyage"` |
    /// `"stub"` (legacy).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Ollama model (default `nomic-embed-text`), used when provider == ollama.
    #[serde(default)]
    pub ollama_model: Option<String>,
    /// Ollama embedding dimension (default 768).
    #[serde(default)]
    pub ollama_dim: Option<usize>,
    /// Ollama base URL (default `http://127.0.0.1:11434`).
    #[serde(default)]
    pub ollama_url: Option<String>,
}

fn default_provider() -> String {
    "local".to_string()
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            ollama_model: None,
            ollama_dim: None,
            ollama_url: None,
        }
    }
}

/// Active embedder status returned by `GET /memory/embedder`.
#[derive(Debug, Serialize)]
pub struct EmbedderStatusResp {
    /// Configured provider (`stub`/`openai`/`voyage`).
    pub provider: String,
    /// Live model id of the active embedder (e.g. `text-embedding-3-small`),
    /// or `null` when vector search is disabled.
    pub model: Option<String>,
    /// Embedding dimension of the active model.
    pub dim: Option<usize>,
    /// True when a vector embedder is active (semantic search works).
    pub active: bool,
    /// True when the provider needs a key and one is resolvable (Keychain/env).
    pub key_present: bool,
}

/// Body of `PUT /memory/embedder`.
#[derive(Debug, Deserialize)]
pub struct SetEmbedderReq {
    pub provider: String,
    /// Optional API key to store in the Keychain for `provider`. When omitted,
    /// the existing stored key (or `<PROVIDER>_API_KEY` env) is used.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub ollama_model: Option<String>,
    #[serde(default)]
    pub ollama_dim: Option<usize>,
    #[serde(default)]
    pub ollama_url: Option<String>,
}

/// Response of `POST /workspaces/{ws}/memory/reindex`.
#[derive(Debug, Serialize)]
pub struct ReindexResp {
    pub embedded: usize,
}

/// Resolve the API key for a remote provider from the Keychain
/// (`<provider>_api_key`) or the `<PROVIDER>_API_KEY` env var.
fn resolve_key(provider: &str, secrets: &Arc<dyn SecretStore>) -> Result<String> {
    let kc = format!("{provider}_api_key");
    if let Ok(Some(k)) = secrets.get(&kc) {
        if !k.trim().is_empty() {
            return Ok(k);
        }
    }
    let env = format!("{}_API_KEY", provider.to_uppercase());
    if let Ok(k) = std::env::var(&env) {
        if !k.trim().is_empty() {
            return Ok(k);
        }
    }
    Err(Error::Invalid(format!(
        "no API key configured for {provider} — provide one via PUT /memory/embedder or set {env}"
    )))
}

/// Build the embedder described by `cfg`. `stub` (and any unknown provider) →
/// the deterministic local stub. `openai`/`voyage` require a resolvable key
/// (else `Err`). The remote provider URLs are trusted constants.
pub fn build_embedder(cfg: &EmbedderConfig, secrets: &Arc<dyn SecretStore>) -> Result<Arc<dyn Embedder>> {
    match cfg.provider.as_str() {
        "openai" => {
            let key = resolve_key("openai", secrets)?;
            Ok(Arc::new(RemoteEmbedder::new(RemoteProvider::Openai, key)))
        }
        "voyage" => {
            let key = resolve_key("voyage", secrets)?;
            Ok(Arc::new(RemoteEmbedder::new(RemoteProvider::Voyage, key)))
        }
        // Real *local neural* embeddings via a localhost Ollama server.
        "ollama" => Ok(Arc::new(OllamaEmbedder::new(
            cfg.ollama_model.clone(),
            cfg.ollama_dim,
            cfg.ollama_url.clone(),
        ))),
        // Legacy zero-dependency stub (FNV bag-of-words).
        "stub" => Ok(Arc::new(StubEmbedder::new(STUB_DIM))),
        // Default: the code-aware deterministic local embedder.
        _ => Ok(Arc::new(LocalCodeEmbedder::default())),
    }
}

/// Apply the persisted embedder setting to the live memory service at daemon
/// boot. A misconfigured remote provider (no key) logs a warning and leaves the
/// default stub embedder in place, so vector search keeps working.
pub async fn apply_configured_embedder(
    memory: &MemoryService,
    settings: &SettingsRepo,
    secrets: &Arc<dyn SecretStore>,
) {
    let cfg: EmbedderConfig = settings
        .get(EMBEDDER_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    // `local` (the code-aware embedder) is already the service default.
    if cfg.provider == "local" {
        return;
    }
    match build_embedder(&cfg, secrets) {
        Ok(e) => {
            tracing::info!("memory: embedder = {} ({})", cfg.provider, e.model_id());
            memory.set_embedder(Some(e));
        }
        Err(e) => {
            tracing::warn!(
                "memory: embedder '{}' not applied ({e}); using local stub",
                cfg.provider
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /memory/embedder` — the active embedder + whether a key is present.
async fn get_embedder(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<EmbedderStatusResp>> {
    let settings = SettingsRepo::new(ctx.pool.clone());
    let cfg: EmbedderConfig = settings
        .get(EMBEDDER_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let status = ctx.memory.embedder_status();
    let key_present = matches!(cfg.provider.as_str(), "openai" | "voyage")
        && resolve_key(&cfg.provider, &ctx.secrets).is_ok();
    Ok(Json(EmbedderStatusResp {
        provider: cfg.provider,
        model: status.as_ref().map(|(m, _)| m.clone()),
        dim: status.as_ref().map(|(_, d)| *d),
        active: status.is_some(),
        key_present,
    }))
}

/// `PUT /memory/embedder` — switch the active embedder. Root only (it stores a
/// provider key and rebuilds vector search for every workspace).
async fn put_embedder(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SetEmbedderReq>,
) -> ApiResult<Json<EmbedderStatusResp>> {
    require_root(&user)?;
    let provider = req.provider.trim().to_string();
    if !matches!(provider.as_str(), "local" | "ollama" | "stub" | "openai" | "voyage") {
        return Err(Error::Invalid(format!("unknown embedder provider: {provider}")).into());
    }
    // Store a freshly-provided key before building (so the build can resolve it).
    if let Some(key) = req.api_key.as_ref().filter(|k| !k.trim().is_empty()) {
        if matches!(provider.as_str(), "openai" | "voyage") {
            ctx.secrets
                .put(&format!("{provider}_api_key"), key.trim())
                .map_err(crate::error::ApiError)?;
        }
    }
    let cfg = EmbedderConfig {
        provider: provider.clone(),
        ollama_model: req.ollama_model.clone(),
        ollama_dim: req.ollama_dim,
        ollama_url: req.ollama_url.clone(),
    };
    // Build first so a missing key is a 400 instead of a silent downgrade.
    let embedder = build_embedder(&cfg, &ctx.secrets).map_err(crate::error::ApiError)?;
    // Persist the choice and swap the live embedder.
    SettingsRepo::new(ctx.pool.clone())
        .put(
            EMBEDDER_KEY,
            &serde_json::to_value(&cfg).unwrap_or_default(),
        )
        .await
        .map_err(crate::error::ApiError)?;
    let (model, dim) = (embedder.model_id().to_string(), embedder.dim());
    ctx.memory.set_embedder(Some(embedder));
    Ok(Json(EmbedderStatusResp {
        provider,
        model: Some(model),
        dim: Some(dim),
        active: true,
        key_present: true,
    }))
}

/// `POST /workspaces/{ws}/memory/reindex` — re-embed the workspace's memories
/// under the active embedder (idempotent, batched).
async fn reindex_memory(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(ws): Path<Id>,
) -> ApiResult<Json<ReindexResp>> {
    crate::auth::require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let embedded = ctx.memory.reindex(&ws).await?;
    Ok(Json(ReindexResp { embedded }))
}

/// Router for the embedder + reindex endpoints (mounted under `/api/v1`).
pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/memory/embedder", get(get_embedder).put(put_embedder))
        .route("/workspaces/{ws}/memory/reindex", post(reindex_memory))
}
