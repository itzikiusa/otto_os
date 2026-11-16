//! Dynamic **model catalog** HTTP routes + `otto-netguard` refresher adapter.
//!
//! Pure CLI probing, HTML token extraction, `models.dev` parsing, and status
//! tracking live in [`otto_sessions::provider_models`]; this module wires them
//! to `otto-netguard`, [`ProviderModelsRepo`], and the `/providers/models` routes.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};
use otto_sessions::provider_models::{
    catalog_last_error, catalog_scrape_target, invalidate_provider_models_cache, probe_catalog_cli,
    set_catalog_fetch_status, CATALOG_STALE_AFTER_HOURS, KNOWN_CATALOG_PROVIDERS,
};
pub use otto_sessions::provider_models::{
    extract_claude_ids, extract_codex_ids, extract_gemini_ids,
    parse_agy_catalog_output as parse_agy_models_output, parse_models_dev,
};
use otto_state::{ProviderModel, ProviderModelsRepo};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::task::JoinHandle;
use tracing::warn;

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::state::ServerCtx;

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const REFRESH_EVERY: Duration = Duration::from_secs(60 * 60);

async fn guarded_fetch_text(url: &str) -> Result<String, String> {
    otto_netguard::check_url(url).await?;
    let client = otto_netguard::guarded_client_builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent("otto-model-catalog")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("{url}: HTTP {}", resp.status()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

async fn docs_scrape(provider: &str) -> Option<Result<Vec<(String, String)>, String>> {
    let (url, extract) = catalog_scrape_target(provider)?;
    Some(match guarded_fetch_text(url).await {
        Err(e) => Err(e),
        Ok(html) => {
            let ids = extract(&html);
            if ids.is_empty() {
                Err(format!("{url}: no model ids extracted"))
            } else {
                Ok(ids.into_iter().map(|id| (id.clone(), id)).collect())
            }
        }
    })
}

async fn catalog_fallback(provider: &str) -> Result<Vec<(String, String)>, String> {
    let body = guarded_fetch_text("https://models.dev/api.json").await?;
    let json: Value = serde_json::from_str(&body).map_err(|e| format!("models.dev: {e}"))?;
    let models = parse_models_dev(&json, provider);
    if models.is_empty() {
        Err(format!("models.dev: no '{provider}' family models"))
    } else {
        Ok(models)
    }
}

fn enabled_catalog_keys(reg: &otto_sessions::ProviderRegistry) -> Vec<String> {
    let mut keys = reg.names();
    for (_slug, family) in reg.model_families() {
        if !keys.contains(&family) {
            keys.push(family);
        }
    }
    keys
}

/// Run the source chain for one provider and persist the first success.
pub async fn refresh_provider(ctx: &ServerCtx, provider: &str) {
    let repo = ProviderModelsRepo::new(ctx.pool.clone());
    let mut errors: Vec<String> = Vec::new();
    let program = ctx
        .manager
        .providers()
        .resolved_family_program(provider)
        .or_else(|| ctx.manager.providers().program_for(provider))
        .unwrap_or_else(|| provider.to_string());

    for source in ["cli", "scrape", "catalog"] {
        let attempt = match source {
            "cli" => probe_catalog_cli(provider, &program, FETCH_TIMEOUT).await,
            "scrape" => docs_scrape(provider).await,
            _ => Some(catalog_fallback(provider).await),
        };
        match attempt {
            None => continue,
            Some(Err(e)) => errors.push(format!("{source}: {e}")),
            Some(Ok(models)) => {
                match repo.upsert_batch(provider, source, &models).await {
                    Ok(()) => {
                        tracing::debug!(
                            provider,
                            source,
                            count = models.len(),
                            "model catalog refreshed"
                        );
                        set_catalog_fetch_status(provider, None);
                    }
                    Err(e) => {
                        warn!(provider, %e, "model catalog persist failed");
                        set_catalog_fetch_status(provider, Some(format!("persist: {e}")));
                    }
                }
                return;
            }
        }
    }
    let joined = errors.join("; ");
    warn!(provider, error = %joined, "model catalog refresh failed (keeping last good list)");
    set_catalog_fetch_status(provider, Some(joined));
}

/// Refresh every enabled known provider sequentially.
pub async fn refresh_all(ctx: &ServerCtx) {
    let enabled = enabled_catalog_keys(ctx.manager.providers());
    for p in KNOWN_CATALOG_PROVIDERS {
        if enabled.iter().any(|e| e == p) {
            refresh_provider(ctx, p).await;
        }
    }
}

/// Hourly background refresher, started with the daemon.
pub fn spawn_refresher(ctx: ServerCtx) -> JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        loop {
            refresh_all(&ctx).await;
            tokio::time::sleep(REFRESH_EVERY).await;
        }
    })
}

#[derive(Serialize)]
pub struct CatalogModel {
    pub id: String,
    pub label: String,
    pub source: String,
}

#[derive(Serialize)]
pub struct ProviderCatalog {
    pub models: Vec<CatalogModel>,
    pub fetched_at: Option<String>,
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Serialize)]
pub struct ModelsResp {
    pub providers: HashMap<String, ProviderCatalog>,
}

#[derive(Deserialize, Default)]
pub struct RefreshReq {
    #[serde(default)]
    pub provider: Option<String>,
}

fn group(rows: Vec<ProviderModel>, enabled: &[String]) -> HashMap<String, ProviderCatalog> {
    let is_enabled = |name: &str| enabled.iter().any(|e| e == name);
    let mut providers: HashMap<String, ProviderCatalog> = HashMap::new();
    for p in KNOWN_CATALOG_PROVIDERS {
        if is_enabled(p) {
            providers.insert(
                p.to_string(),
                ProviderCatalog {
                    models: Vec::new(),
                    fetched_at: None,
                    stale: true,
                    last_error: catalog_last_error(p),
                },
            );
        }
    }
    for row in rows {
        if !is_enabled(&row.provider) {
            continue;
        }
        let entry = providers
            .entry(row.provider.clone())
            .or_insert_with(|| ProviderCatalog {
                models: Vec::new(),
                fetched_at: None,
                stale: true,
                last_error: catalog_last_error(&row.provider),
            });
        if row.source != "manual"
            && entry
                .fetched_at
                .as_deref()
                .is_none_or(|cur| row.fetched_at.as_str() > cur)
        {
            entry.fetched_at = Some(row.fetched_at.clone());
        }
        entry.models.push(CatalogModel {
            id: row.model_id,
            label: row.label,
            source: row.source,
        });
    }
    for cat in providers.values_mut() {
        cat.stale = match cat.fetched_at.as_deref().and_then(|t| {
            DateTime::parse_from_rfc3339(t)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        }) {
            Some(at) => Utc::now() - at > chrono::Duration::hours(CATALOG_STALE_AFTER_HOURS),
            None => true,
        };
    }
    providers
}

async fn payload(ctx: &ServerCtx) -> ApiResult<ModelsResp> {
    let rows = ProviderModelsRepo::new(ctx.pool.clone()).list_all().await?;
    let enabled = enabled_catalog_keys(ctx.manager.providers());
    Ok(ModelsResp {
        providers: group(rows, &enabled),
    })
}

/// `GET /api/v1/providers/models` — the full grouped catalog + staleness.
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<ModelsResp>> {
    Ok(Json(payload(&ctx).await?))
}

/// `POST /api/v1/providers/models/refresh` — run the source chain now
/// (optionally for one provider), invalidate `/meta`'s cache, and return the
/// updated catalog.
pub async fn refresh(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
    body: Option<Json<RefreshReq>>,
) -> ApiResult<Json<ModelsResp>> {
    let req = body.map(|Json(r)| r).unwrap_or_default();
    let target = req
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty());
    match target {
        Some(p) => refresh_provider(&ctx, p).await,
        None => refresh_all(&ctx).await,
    }
    invalidate_provider_models_cache(target);
    Ok(Json(payload(&ctx).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE_HTML: &str = include_str!("../tests/fixtures/claude_models.html");
    const CODEX_HTML: &str = include_str!("../tests/fixtures/codex_models.html");
    const MODELS_DEV: &str = include_str!("../tests/fixtures/models_dev.json");
    const AGY_OUT: &str = include_str!("../tests/fixtures/agy_models.txt");

    #[test]
    fn claude_ids_from_docs_snapshot() {
        let ids = extract_claude_ids(CLAUDE_HTML);
        for expect in [
            "claude-fable-5",
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-haiku-4-5",
        ] {
            assert!(
                ids.contains(&expect.to_string()),
                "missing {expect} in {ids:?}"
            );
        }
        for reject in ids.iter() {
            assert!(!reject.contains("system-card"), "slug leaked: {reject}");
            assert!(!reject.ends_with("-and"), "slug leaked: {reject}");
        }
    }

    #[test]
    fn codex_ids_from_docs_snapshot() {
        let ids = extract_codex_ids(CODEX_HTML);
        for expect in ["gpt-5.3-codex", "gpt-5.4", "gpt-5.6-luna"] {
            assert!(
                ids.contains(&expect.to_string()),
                "missing {expect} in {ids:?}"
            );
        }
        assert!(
            ids.iter()
                .all(|i| !i.ends_with(".jpg") && !i.ends_with(".webp")),
            "asset name leaked: {ids:?}"
        );
    }

    #[test]
    fn codex_raw_fallback_filters_assets() {
        let html = r#"see "gpt-5.4" and img gpt-5.4.jpg plus gpt-staging.com and gpt-5.5"#;
        let ids = extract_codex_ids(html);
        assert_eq!(ids, vec!["gpt-5.4".to_string(), "gpt-5.5".to_string()]);
    }

    #[test]
    fn agy_cli_output_parses() {
        let models = parse_agy_models_output(AGY_OUT);
        assert!(!models.is_empty());
        assert!(
            models.iter().any(|(id, _)| id.starts_with("gemini-")),
            "{models:?}"
        );
        let (_, label) = models.iter().find(|(id, _)| id.contains("pro")).unwrap();
        assert!(label.contains(' '), "label should be human text: {label}");
    }

    #[test]
    fn gemini_ids_token_scan() {
        let html =
            r#"<code>gemini-3.1-pro-high</code> and "gemini-3.7-flash-low" but not gemini-api"#;
        let ids = extract_gemini_ids(html);
        assert_eq!(
            ids,
            vec![
                "gemini-3.1-pro-high".to_string(),
                "gemini-3.7-flash-low".to_string()
            ]
        );
    }

    #[test]
    fn models_dev_snapshot_parses_per_family() {
        let json: Value = serde_json::from_str(MODELS_DEV).unwrap();
        let claude = parse_models_dev(&json, "claude");
        assert!(claude.iter().all(|(id, _)| id.starts_with("claude-")));
        assert!(!claude.is_empty());
        let agy = parse_models_dev(&json, "agy");
        assert!(
            agy.iter().all(|(id, _)| id.starts_with("gemini-")),
            "{agy:?}"
        );
        assert!(!agy.is_empty());
        assert_eq!(agy, parse_models_dev(&json, "gemini"));
        let opencode = parse_models_dev(&json, "opencode");
        assert!(!opencode.is_empty());
        assert!(opencode
            .iter()
            .any(|(id, _)| id.starts_with("anthropic/claude-")));
        assert!(opencode.iter().any(|(id, _)| id.starts_with("openai/gpt-")));
        assert!(opencode
            .iter()
            .any(|(id, _)| id.starts_with("google/gemini-")));
        assert_eq!(opencode, parse_models_dev(&json, "pi"));
        assert!(parse_models_dev(&json, "unknown").is_empty());
        assert!(parse_models_dev(&Value::Null, "claude").is_empty());
    }

    #[test]
    fn group_excludes_disabled_providers_and_includes_all_seven_enabled_builtins() {
        let rows = vec![
            ProviderModel {
                provider: "claude".into(),
                model_id: "claude-sonnet-5".into(),
                label: "Sonnet 5".into(),
                source: "scrape".into(),
                fetched_at: Utc::now().to_rfc3339(),
                notes: String::new(),
            },
            ProviderModel {
                provider: "opencode".into(),
                model_id: "openai/gpt-5".into(),
                label: "GPT-5".into(),
                source: "cli".into(),
                fetched_at: Utc::now().to_rfc3339(),
                notes: String::new(),
            },
        ];
        let enabled = vec![
            "claude".to_string(),
            "gemini".to_string(),
            "grok".to_string(),
            "pi".to_string(),
        ];
        let grouped = group(rows, &enabled);
        assert!(grouped.contains_key("claude"));
        assert!(grouped.contains_key("gemini"));
        assert!(grouped.contains_key("grok"));
        assert!(grouped.contains_key("pi"));
        assert!(
            !grouped.contains_key("opencode"),
            "disabled provider filtered"
        );
        assert!(!grouped.contains_key("codex"), "disabled builtin filtered");
    }
}
