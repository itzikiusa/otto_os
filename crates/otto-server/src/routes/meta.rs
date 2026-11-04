//! Endpoints #1 (health) and #2 (meta).

use std::time::Duration;

use axum::extract::State;
use axum::Json;
use otto_core::api::MetaResp;
use otto_state::{SettingsRepo, UsersRepo};
use serde_json::{json, Value};

use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Current API contract version.
pub const API_VERSION: u32 = 1;

/// `GET /api/v1/health`
pub async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// Whether the request carries a bearer that verifies as a full (unscoped,
/// non-MCP) account credential. A share link — verified or still OTP-pending —
/// and an MCP token are NOT "signed in" here (S8-308): a leaked share link
/// must not learn tool/provider versions or the second loopback base.
async fn caller_authenticated(ctx: &ServerCtx, headers: &axum::http::HeaderMap) -> bool {
    let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return false;
    };
    ctx.authenticator
        .authenticate(token)
        .await
        .is_ok_and(|auth| !auth.is_scoped() && !auth.mcp_only)
}

/// `GET /api/v1/meta`
///
/// Public (the login / onboarding screens need it), but an UNAUTHENTICATED
/// caller once onboarding is done gets only `{version, api_version,
/// needs_onboarding}` — tool and provider versions, the listener flag and the
/// second loopback base are for signed-in callers (S8-04: this route is
/// reachable by anyone through the tunnel's public hostname). Before
/// onboarding there is no account to protect, so the first-run screen still
/// sees the tool checks.
pub async fn meta(
    State(ctx): State<ServerCtx>,
    headers: axum::http::HeaderMap,
) -> ApiResult<Json<MetaResp>> {
    let needs_onboarding = UsersRepo::new(ctx.pool.clone()).count().await? == 0;
    if !needs_onboarding && !caller_authenticated(&ctx, &headers).await {
        return Ok(Json(MetaResp {
            version: ctx.version.clone(),
            api_version: API_VERSION,
            needs_onboarding,
            network_listener: false,
            tools: Vec::new(),
            providers: Vec::new(),
            default_provider: None,
            model_flags: Default::default(),
            provider_models: Default::default(),
            alt_loopback_base: None,
        }));
    }

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
        .get(otto_state::settings::DEFAULT_PROVIDER_KEY)
        .await?
        .as_ref()
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .filter(|s| providers.iter().any(|p| p == s));

    // Probe the configured executable, not merely a provider's display name.
    // This makes custom providers and builtin command overrides first-class in
    // every launch readiness surface without executing custom version commands.
    let tool_specs = otto_sessions::tool_probe::tool_specs(
        providers
            .iter()
            .map(|name| (name.as_str(), ctx.manager.providers().program_for(name))),
    );
    let tools = otto_sessions::tool_probe::probe_tools_cached(tool_specs.clone()).await;
    // A family's `models` subcommand runs through the first INSTALLED
    // provider of that family (a custom `grok` provider is the grok CLI).
    let families = ctx.manager.providers().model_families();
    let program_of = |family: &str| {
        families
            .iter()
            .filter(|(_, f)| f == family)
            .find(|(p, _)| tools.iter().any(|t| &t.name == p && t.found))
            .and_then(|(p, _)| tool_specs.get(p).cloned())
    };
    let probe = otto_sessions::provider_models::ModelProbe {
        agy_program: program_of("agy"),
        grok_program: program_of("grok"),
        opencode_program: program_of("opencode"),
        pi_program: program_of("pi"),
        families,
    };
    let provider_models =
        otto_sessions::provider_models::probe_provider_models_cached(&probe).await;

    Ok(Json(MetaResp {
        version: ctx.version.clone(),
        api_version: API_VERSION,
        needs_onboarding,
        network_listener,
        tools,
        providers,
        default_provider,
        model_flags,
        provider_models,
        alt_loopback_base: crate::transport::alt_loopback_base(),
    }))
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
    // Parse, then rebuild from the parsed parts: the request only ever
    // carries an `https://github.com/<path>` URL — never the caller's raw
    // string, and never another scheme / host / port.
    let parsed = reqwest::Url::parse(q.url.trim())
        .ok()
        .filter(|u| {
            u.scheme() == "https" && u.host_str() == Some("github.com") && u.port().is_none()
        })
        .ok_or_else(|| {
            ApiError(otto_core::Error::Invalid(
                "only github.com walkthrough assets can be resolved".into(),
            ))
        })?;
    let mut safe = reqwest::Url::parse(WALKTHROUGH_ORIGIN).expect("static origin parses");
    safe.set_path(parsed.path());
    safe.set_query(parsed.query());
    let url = resolve_one_hop(safe.as_str())
        .await
        .map_err(|e| ApiError(otto_core::Error::Upstream(e)))?;
    Ok(Json(ResolveWalkthroughResp { url }))
}

/// HEAD `url` without following redirects; return its `Location` (only when
/// it is an https URL) or `url` itself for a non-redirect answer.
async fn resolve_one_hop(url: &str) -> Result<String, String> {
    otto_netguard::check_url(url).await?;
    // Guarded resolver: dial only the vetted address (no DNS rebinding).
    let client = otto_netguard::guarded_client_builder()
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

    /// S8-308: only a full account credential counts as "signed in" for
    /// `/meta`'s detail — never a share link or an MCP token.
    #[tokio::test]
    async fn share_and_mcp_tokens_are_not_signed_in_for_meta() {
        let pool = crate::test_support::mem_pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
        let owner = otto_state::UsersRepo::new(pool.clone())
            .create("s8308", "x", "Owner", true)
            .await
            .unwrap();
        let repo = otto_rbac::AuthRepo::new(pool.clone());
        let login = repo.issue(&owner.id).await.unwrap();
        let (share, _) = repo
            .issue_share_token(
                &owner.id,
                &otto_core::Id::from("S1"),
                otto_core::domain::WorkspaceRole::Viewer,
                3600,
                None,
            )
            .await
            .unwrap();
        let mcp = repo.issue_mcp_token(&owner.id, None).await.unwrap();
        let headers = |t: &str| {
            let mut h = axum::http::HeaderMap::new();
            h.insert(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {t}").parse().unwrap(),
            );
            h
        };
        assert!(caller_authenticated(&ctx, &headers(&login)).await);
        assert!(!caller_authenticated(&ctx, &headers(&share)).await);
        assert!(!caller_authenticated(&ctx, &headers(&mcp)).await);
    }

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
        assert!(
            url.starts_with("https://release-assets.githubusercontent.com/"),
            "{url}"
        );
    }
}
