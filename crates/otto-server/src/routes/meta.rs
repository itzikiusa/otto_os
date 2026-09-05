//! Endpoints #1 (health) and #2 (meta).

use std::time::Duration;

use axum::extract::State;
use axum::Json;
use otto_core::api::{MetaResp, ToolStatus};
use otto_state::{SettingsRepo, UsersRepo};
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::timeout;

use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Current API contract version.
pub const API_VERSION: u32 = 1;

const DETECTED_TOOLS: [&str; 5] = ["claude", "codex", "agy", "git", "clickhouse"];

/// `GET /api/v1/health`
pub async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// `GET /api/v1/meta`
pub async fn meta(State(ctx): State<ServerCtx>) -> ApiResult<Json<MetaResp>> {
    let needs_onboarding = UsersRepo::new(ctx.pool.clone()).count().await? == 0;

    let settings = SettingsRepo::new(ctx.pool.clone());
    let network_listener = settings
        .get("network_listener")
        .await?
        .and_then(|v| v.get("enabled").and_then(Value::as_bool))
        .unwrap_or(false);

    // The live registry (builtins + custom overrides) is the single source
    // of truth for the provider list.
    let providers = ctx.manager.providers().names();
    // Per-provider: whether the CLI accepts a model flag (its spec carries a
    // `model_args` template). Pickers hide the model control when false, so a
    // pinned model is never silently dropped.
    let model_flags = providers
        .iter()
        .map(|p| (p.clone(), ctx.manager.providers().supports_model(p)))
        .collect();

    // The configured default agent (a provider name). Used by the UI to
    // preselect a provider for new sessions, and mirrors the value channel
    // replies fall back to. Stored as a bare JSON string; empty => unset.
    // A default that names a now-DISABLED provider is reported as unset, so the
    // UI never labels an excluded provider as the default (and doesn't disagree
    // with what the daemon would actually spawn).
    let default_provider = settings
        .get("default_provider")
        .await?
        .as_ref()
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .filter(|s| providers.iter().any(|p| p == s));

    let tools = futures_util::future::join_all(DETECTED_TOOLS.iter().map(|t| detect_tool(t))).await;

    Ok(Json(MetaResp {
        version: ctx.version.clone(),
        api_version: API_VERSION,
        needs_onboarding,
        network_listener,
        tools,
        providers,
        default_provider,
        model_flags,
    }))
}

/// Probe one external tool: `which <name>` for presence, then
/// `<name> --version` (2s timeout) for the version string.
async fn detect_tool(name: &str) -> ToolStatus {
    let found = match timeout(
        Duration::from_secs(2),
        Command::new("which").arg(name).output(),
    )
    .await
    {
        Ok(Ok(out)) => out.status.success(),
        _ => false,
    };

    let mut version = None;
    if found {
        if let Ok(Ok(out)) = timeout(
            Duration::from_secs(2),
            Command::new(name).arg("--version").output(),
        )
        .await
        {
            if out.status.success() {
                version = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty());
            }
        }
    }

    ToolStatus {
        name: name.to_string(),
        found,
        version,
    }
}

// ---------------------------------------------------------------------------
// Walkthrough video redirect resolver
// ---------------------------------------------------------------------------

/// The only origin the resolver will touch: walkthrough MP4s are assets of the
/// rolling `walkthroughs` GitHub release. GitHub answers with a 302 to a
/// short-lived signed `release-assets.githubusercontent.com` URL — and WebKit
/// (the desktop webview) refuses a `<video src>` that redirects
/// (`MEDIA_ERR_SRC_NOT_SUPPORTED`), while Chromium tolerates it. Resolving the
/// hop here lets the page point the element straight at the final URL.
const WALKTHROUGH_ORIGIN: &str = "https://github.com/";

#[derive(serde::Deserialize)]
pub struct ResolveWalkthroughQuery {
    pub url: String,
}

#[derive(serde::Serialize)]
pub struct ResolveWalkthroughResp {
    /// Final, directly-playable URL (after following GitHub's redirect), or
    /// the input unchanged when it did not redirect.
    pub url: String,
}

/// `GET /api/v1/walkthroughs/resolve?url=` — follow the GitHub release-asset
/// redirect (one hop, HEAD, no body) and return the final URL. Refuses any
/// URL outside `https://github.com/` so this can never become an open
/// redirect prober.
pub async fn resolve_walkthrough(
    axum::extract::Query(q): axum::extract::Query<ResolveWalkthroughQuery>,
) -> ApiResult<Json<ResolveWalkthroughResp>> {
    if !q.url.starts_with(WALKTHROUGH_ORIGIN) {
        return Err(ApiError(otto_core::Error::Invalid(
            "only github.com walkthrough assets can be resolved".into(),
        )));
    }
    let url = resolve_one_hop(&q.url).await.map_err(|e| ApiError(otto_core::Error::Upstream(e)))?;
    Ok(Json(ResolveWalkthroughResp { url }))
}

/// HEAD `url` without following redirects; return its `Location` (only when
/// it is an https URL) or `url` itself for a non-redirect answer.
async fn resolve_one_hop(url: &str) -> Result<String, String> {
    otto_netguard::check_url(url).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("otto-walkthroughs")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.head(url).send().await.map_err(|e| e.to_string())?;
    if resp.status().is_redirection() {
        let loc = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| "redirect without Location".to_string())?;
        if !loc.starts_with("https://") {
            return Err("redirect target is not https".into());
        }
        return Ok(loc.to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("{url}: HTTP {}", resp.status()));
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod walkthrough_tests {
    use super::*;

    #[tokio::test]
    async fn rejects_non_github_urls() {
        let r = resolve_walkthrough(axum::extract::Query(ResolveWalkthroughQuery {
            url: "https://example.com/x.mp4".into(),
        }))
        .await;
        assert!(matches!(r, Err(ApiError(otto_core::Error::Invalid(_)))));
    }

    /// Network: follows GitHub's 302 to the signed release-assets URL.
    #[tokio::test]
    #[ignore = "needs network"]
    async fn resolves_github_release_asset() {
        let url = resolve_one_hop(
            "https://github.com/itzikiusa/otto_os/releases/download/walkthroughs/Intro.mp4",
        )
        .await
        .unwrap();
        assert!(url.starts_with("https://release-assets.githubusercontent.com/"), "{url}");
    }
}
