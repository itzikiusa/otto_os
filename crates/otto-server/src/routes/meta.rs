//! Endpoints #1 (health) and #2 (meta).

use std::time::Duration;

use axum::extract::State;
use axum::Json;
use otto_core::api::{MetaResp, ToolStatus};
use otto_state::{SettingsRepo, UsersRepo};
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::timeout;

use crate::error::ApiResult;
use crate::state::ServerCtx;

/// Current API contract version.
pub const API_VERSION: u32 = 1;

const DETECTED_TOOLS: [&str; 4] = ["claude", "codex", "agy", "git"];

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

    // The configured default agent (a provider name). Used by the UI to
    // preselect a provider for new sessions, and mirrors the value channel
    // replies fall back to. Stored as a bare JSON string; empty => unset.
    let default_provider = settings
        .get("default_provider")
        .await?
        .as_ref()
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty());

    let tools = futures_util::future::join_all(DETECTED_TOOLS.iter().map(|t| detect_tool(t))).await;

    Ok(Json(MetaResp {
        version: ctx.version.clone(),
        api_version: API_VERSION,
        needs_onboarding,
        network_listener,
        tools,
        providers,
        default_provider,
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
