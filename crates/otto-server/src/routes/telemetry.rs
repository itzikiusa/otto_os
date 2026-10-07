//! Root-only machine diagnostics; telemetry never contains arbitrary UI text.
use crate::{auth::require_root, ApiError, ApiResult, CurrentUser, ServerCtx};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use otto_core::Error;
use otto_state::SettingsRepo;
use otto_telemetry::{
    NativeProfile, SpanRecord, Suggestion, TelemetryConfig, TelemetryOverview, TelemetryService,
    TelemetryStatus,
};
use serde::Deserialize;
use std::sync::Arc;

const KEY: &str = "application_telemetry";
static CONFIG_WRITE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn service(ctx: &ServerCtx) -> ApiResult<&Arc<TelemetryService>> {
    ctx.telemetry.as_ref().ok_or_else(|| {
        ApiError(Error::Upstream(
            "Application telemetry is unavailable".into(),
        ))
    })
}
fn upstream(e: impl std::fmt::Display) -> ApiError {
    ApiError(Error::Upstream(e.to_string()))
}

pub async fn config(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<TelemetryConfig>> {
    require_root(&user)?;
    Ok(Json(service(&ctx)?.config()))
}
pub async fn put_config(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(config): Json<TelemetryConfig>,
) -> ApiResult<Json<TelemetryConfig>> {
    require_root(&user)?;
    config
        .validate()
        .map_err(|e| ApiError(Error::Invalid(e.to_string())))?;
    let _serial = CONFIG_WRITE.lock().await;
    let svc = service(&ctx)?;
    let value = serde_json::to_value(&config).map_err(upstream)?;
    SettingsRepo::new(ctx.pool.clone()).put(KEY, &value).await?;
    svc.configure(config.clone()).await.map_err(upstream)?;
    Ok(Json(config))
}
pub async fn status(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<TelemetryStatus>> {
    require_root(&user)?;
    Ok(Json(service(&ctx)?.status()))
}
#[derive(Deserialize)]
pub struct Window {
    pub hours: Option<u32>,
}
pub async fn overview(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<Window>,
) -> ApiResult<Json<TelemetryOverview>> {
    require_root(&user)?;
    Ok(Json(
        service(&ctx)?
            .overview(q.hours.unwrap_or(24).clamp(1, 720))
            .await
            .map_err(upstream)?,
    ))
}
pub async fn suggestions(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<Suggestion>>> {
    require_root(&user)?;
    Ok(Json(service(&ctx)?.suggestions()))
}
pub async fn analyze(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<Suggestion>>> {
    require_root(&user)?;
    Ok(Json(service(&ctx)?.analyze().await.map_err(upstream)?))
}
pub async fn dismiss(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_root(&user)?;
    service(&ctx)?.dismiss(&id).await.map_err(upstream)?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn trace(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<SpanRecord>>> {
    require_root(&user)?;
    Ok(Json(service(&ctx)?.trace(&id).await.map_err(upstream)?))
}
pub async fn latest_profile(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Option<NativeProfile>>> {
    require_root(&user)?;
    Ok(Json(service(&ctx)?.latest_profile()))
}
pub async fn profile(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<NativeProfile>> {
    require_root(&user)?;
    Ok(Json(service(&ctx)?.profile().await.map_err(upstream)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ingest {
    pub spans: Vec<SpanRecord>,
}

fn validate_ui(spans: &[SpanRecord]) -> ApiResult<()> {
    if spans.len() > 100 {
        return Err(ApiError(Error::Invalid(
            "A telemetry batch may contain at most 100 spans".into(),
        )));
    }
    for span in spans {
        span.validate()
            .map_err(|e| ApiError(Error::Invalid(e.to_string())))?;
        if !matches!(
            span.name.as_str(),
            "ui.navigation"
                | "ui.chunk"
                | "ui.render"
                | "ui.long_task"
                | "ui.frame_delay"
                | "http.client"
                | "ui.request.queue"
                | "ui.response.decode"
        ) || !matches!(span.kind.as_str(), "client" | "internal")
            || !UI_COMPONENTS.contains(&span.component.as_str())
            || span.attributes.contains_key("http.route")
        {
            return Err(ApiError(Error::Invalid(
                "Unsupported UI telemetry operation".into(),
            )));
        }
    }
    Ok(())
}
const UI_COMPONENTS: &[&str] = &[
    "agents",
    "workbench",
    "connections",
    "history",
    "assistant",
    "home",
    "git",
    "database",
    "api",
    "vault",
    "mission-control",
    "rooms",
    "workflows",
    "scheduled-tasks",
    "personal-agents",
    "loops",
    "swarm",
    "product",
    "design",
    "canvas",
    "browser",
    "run-with-otto",
    "mcp",
    "brokers",
    "kubernetes",
    "aws",
    "insights",
    "usage",
    "skills-eval",
    "proof",
    "settings",
    "walkthroughs",
    "plugin",
    "snip",
    "shell",
    "other",
];
pub async fn ingest(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(batch): Json<Ingest>,
) -> ApiResult<Json<serde_json::Value>> {
    require_root(&user)?;
    validate_ui(&batch.spans)?;
    let svc = service(&ctx)?;
    if !svc.enabled() {
        return Ok(Json(serde_json::json!({"accepted":0})));
    }
    let accepted = batch
        .spans
        .into_iter()
        .map(|span| usize::from(svc.record(span)))
        .sum::<usize>();
    Ok(Json(serde_json::json!({"accepted":accepted})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn producer_contract_accepts_mixed_batch_and_rejects_private_names() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../otto-telemetry/fixtures/ui-ingest.json"
        ))
        .unwrap();
        let mut spans = vec![
            SpanRecord::new("ui.navigation", "git", 12.0),
            SpanRecord::new("ui.render", "git", 2.0),
        ];
        for request in fixture["requests"].as_array().unwrap() {
            let mut span = SpanRecord::new(
                request["name"].as_str().unwrap(),
                request["component"].as_str().unwrap(),
                1.0,
            );
            span.kind = "client".into();
            spans.push(span);
        }
        assert!(validate_ui(&spans).is_ok());
        for name in fixture["rejected_names"].as_array().unwrap() {
            spans.last_mut().unwrap().name = name.as_str().unwrap().into();
            assert!(validate_ui(&spans).is_err(), "accepted {name}");
        }
        spans.last_mut().unwrap().name = "x".repeat(97);
        assert!(validate_ui(&spans).is_err());
        spans.last_mut().unwrap().name = "http.client".into();
        spans.last_mut().unwrap().component = "private-project".into();
        assert!(validate_ui(&spans).is_err());
    }
    #[test]
    fn ingestion_rejects_dynamic_names_and_routes_atomically() {
        let mut span = SpanRecord::new("ui.navigation", "agents", 12.0);
        assert!(validate_ui(&[span.clone()]).is_ok());
        span.name = "private-project-name".into();
        assert!(validate_ui(&[span.clone()]).is_err());
        span.name = "ui.navigation".into();
        span.attributes
            .insert("http.route".into(), "/secrets".into());
        assert!(validate_ui(&[span]).is_err());
        assert!(validate_ui(&vec![SpanRecord::new("ui.navigation", "agents", 1.0); 101]).is_err());
    }
}
