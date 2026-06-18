//! Usage & metrics endpoints, backed by the embedded ClickHouse engine.
//!
//! Read/admin routes (`/usage/...`) are root-only — the dashboard aggregates
//! across every workspace, mirroring the daemon-wide settings panel. The
//! `/ingest/usage` route is unauthenticated but gated by the per-session token
//! Otto sets on the agent PTY, so injected provider hooks can report token
//! usage without a user bearer token.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use otto_core::Id;
use otto_state::SettingsRepo;
use otto_usage::{MetricPoint, UsageConfig, UsageEvent, UsageStatus, UsageSummary};
use serde::Deserialize;
use serde_json::Value;

use crate::auth::{require_root, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// `settings` key the usage config is persisted under.
const SETTINGS_KEY: &str = "usage";

fn load_config(ctx: &ServerCtx) -> impl std::future::Future<Output = UsageConfig> + '_ {
    async move {
        SettingsRepo::new(ctx.pool.clone())
            .get(SETTINGS_KEY)
            .await
            .ok()
            .flatten()
            .as_ref()
            .map(|v| UsageConfig::from_json(Some(v)))
            .unwrap_or_default()
    }
}

async fn save_config(ctx: &ServerCtx, cfg: &UsageConfig) -> Result<(), ApiError> {
    let value = serde_json::to_value(cfg)
        .map_err(|e| ApiError(otto_core::Error::Internal(format!("serialize usage config: {e}"))))?;
    SettingsRepo::new(ctx.pool.clone())
        .put(SETTINGS_KEY, &value)
        .await
        .map_err(ApiError)
}

#[derive(Debug, Deserialize)]
pub struct WindowDays {
    /// Days of history to roll up (default 30).
    pub days: Option<u32>,
    /// When true (default), exclude externally-recorded (non-Otto) sessions.
    pub otto_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct WindowMinutes {
    /// Minutes of metrics history (default 60).
    pub minutes: Option<u32>,
}

/// `GET /usage/status` — engine + ClickHouse health (root).
pub async fn status(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<UsageStatus>> {
    require_root(&user)?;
    Ok(Json(ctx.usage.status().await))
}

/// `GET /usage/summary?days=N` — provider/day/session rollups (root).
pub async fn summary(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<WindowDays>,
) -> ApiResult<Json<UsageSummary>> {
    require_root(&user)?;
    let days = q.days.unwrap_or(30).clamp(1, 3650);
    let otto_only = q.otto_only.unwrap_or(true);
    let mut summary = ctx.usage.summary(days, otto_only).await.map_err(ApiError)?;
    enrich_sessions(&ctx, &mut summary.sessions).await;
    Ok(Json(summary))
}

/// Enrich top-session rows with the Otto session title (pane name), kind
/// (review / product / channel / agent…), and workspace name — looked up from
/// SQLite. External (non-Otto) sessions have no matching row and are left as-is.
async fn enrich_sessions(ctx: &ServerCtx, sessions: &mut [otto_usage::SessionUsage]) {
    let repo = otto_state::SessionsRepo::new(ctx.pool.clone());
    let mut ws_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for s in sessions.iter_mut() {
        let Ok(sess) = repo.get(&s.session_id).await else {
            continue; // external / unknown session
        };
        let title = sess.title.trim();
        if !title.is_empty() {
            s.title = Some(title.to_string());
        }
        s.kind = Some(session_kind_label(&sess));
        let wsid = sess.workspace_id.clone();
        if let Some(name) = ws_names.get(&wsid) {
            s.workspace_name = Some(name.clone());
        } else if let Ok(w) = ctx.workspaces.get(&wsid).await {
            ws_names.insert(wsid, w.name.clone());
            s.workspace_name = Some(w.name);
        }
    }
}

/// Derive a short usage-kind label for an Otto session: prefer the meta `source`
/// tag set by the review/product/channel runners, else fall back to the session
/// kind.
fn session_kind_label(s: &otto_core::domain::Session) -> String {
    if let Some(src) = s.meta.get("source").and_then(Value::as_str) {
        return match src {
            "product-analysis" => "product".to_string(),
            other => other.to_string(), // "review", "channel"
        };
    }
    format!("{:?}", s.kind).to_lowercase()
}

/// `GET /usage/metrics?minutes=N` — system metrics time-series (root).
pub async fn metrics(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<WindowMinutes>,
) -> ApiResult<Json<Vec<MetricPoint>>> {
    require_root(&user)?;
    let minutes = q.minutes.unwrap_or(60).clamp(1, 60 * 24 * 30);
    Ok(Json(ctx.usage.metrics(minutes).await.map_err(ApiError)?))
}

#[derive(Debug, Deserialize)]
pub struct UsageConfigReq {
    pub enabled: Option<bool>,
    pub retention_days: Option<u32>,
    pub metrics_interval_secs: Option<u64>,
    pub clickhouse_path: Option<String>,
}

/// `PUT /usage/config` — update + persist config and apply it live (root).
pub async fn put_config(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UsageConfigReq>,
) -> ApiResult<Json<UsageStatus>> {
    require_root(&user)?;

    let mut cfg = load_config(&ctx).await;
    if let Some(v) = req.enabled {
        cfg.enabled = v;
    }
    if let Some(v) = req.retention_days {
        cfg.retention_days = v.clamp(1, 3650);
    }
    if let Some(v) = req.metrics_interval_secs {
        cfg.metrics_interval_secs = v.clamp(5, 3600);
    }
    if let Some(v) = req.clickhouse_path {
        cfg.clickhouse_path = Some(v).filter(|s| !s.trim().is_empty());
    }

    save_config(&ctx, &cfg).await?;
    // Bring the engine up/down for enabled+path changes, then apply TTL (an
    // ALTER — CREATE IF NOT EXISTS won't change an existing table's TTL) and the
    // sampling interval.
    ctx.usage.reinit(cfg.clone()).await;
    ctx.usage.set_metrics_interval(cfg.metrics_interval_secs);
    let _ = ctx.usage.set_retention(cfg.retention_days).await;
    Ok(Json(ctx.usage.status().await))
}

/// `POST /usage/install` — install/update ClickHouse via the official
/// installer, then activate the engine and persist the resolved path (root).
/// The download is large, so this can take a while.
pub async fn install(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<UsageStatus>> {
    require_root(&user)?;
    let bin = ctx.usage.install_clickhouse().await.map_err(ApiError)?;

    let mut cfg = load_config(&ctx).await;
    cfg.enabled = true;
    cfg.clickhouse_path = Some(bin.display().to_string());
    save_config(&ctx, &cfg).await?;
    let _ = ctx.usage.set_retention(cfg.retention_days).await;
    Ok(Json(ctx.usage.status().await))
}

#[derive(Debug, Default, Deserialize)]
pub struct IngestUsageReq {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub duration_ms: u64,
}

/// `POST /ingest/usage` — record a token-usage event for the session named in
/// `X-Otto-Session` (verified against the per-session ingest token). Cost is
/// estimated from the model + tokens when not supplied. Always 204.
pub async fn ingest(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(req): Json<IngestUsageReq>,
) -> StatusCode {
    let Some(sid) = headers
        .get("x-otto-session")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
    else {
        return StatusCode::NO_CONTENT;
    };
    let token = headers
        .get("x-otto-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !ctx.manager.verify_ingest_token(&sid, token) {
        return StatusCode::NO_CONTENT;
    }
    let Ok(session) = ctx.manager.get(&sid).await else {
        return StatusCode::NO_CONTENT;
    };

    let cost = if req.cost_usd > 0.0 {
        req.cost_usd
    } else {
        otto_usage::estimate_cost(&req.model, req.input_tokens, req.output_tokens)
    };
    ctx.usage.record(UsageEvent {
        workspace_id: session.workspace_id,
        session_id: session.id,
        provider: session.provider,
        model: req.model,
        kind: if req.kind.is_empty() {
            "completion".into()
        } else {
            req.kind
        },
        input_tokens: req.input_tokens,
        output_tokens: req.output_tokens,
        cache_read_tokens: req.cache_read_tokens,
        cache_write_tokens: req.cache_write_tokens,
        cost_usd: cost,
        duration_ms: req.duration_ms,
    });
    StatusCode::NO_CONTENT
}

/// Map a session's activity-trail entry to a usage row, mining token counts /
/// model / cost from the entry's `detail` payload when present. Returns `None`
/// for noise (task-tracker churn, human notes). This is the automatic path —
/// every meaningful agent action becomes a usage event (with tokens when the
/// provider reports them, otherwise an activity count).
pub fn trail_to_usage(
    workspace_id: &Id,
    session_id: &Id,
    provider: &str,
    event: &otto_core::domain::TrailEvent,
) -> Option<UsageEvent> {
    use otto_core::domain::TrailKind;

    let kind = match event.kind {
        TrailKind::Prompt => "prompt",
        TrailKind::Command => "command",
        TrailKind::Skill => "skill",
        TrailKind::Tool => "tool",
        TrailKind::File => "file",
        TrailKind::Web => "web",
        TrailKind::Session => "session",
        TrailKind::Other => "other",
        // Task-tracker updates and human notes aren't agent "usage".
        TrailKind::Task | TrailKind::Note => return None,
    };

    let detail = event.detail.as_ref();
    let usage = detail.and_then(|d| d.get("usage"));
    let num = |obj: Option<&Value>, key: &str| -> u64 {
        obj.and_then(|o| o.get(key)).and_then(Value::as_u64).unwrap_or(0)
    };
    let input = num(usage, "input_tokens");
    let output = num(usage, "output_tokens");
    let cache_read = num(usage, "cache_read_input_tokens").max(num(usage, "cache_read_tokens"));
    let cache_write =
        num(usage, "cache_creation_input_tokens").max(num(usage, "cache_write_tokens"));
    let model = detail
        .and_then(|d| d.get("model"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut cost = detail
        .and_then(|d| d.get("cost_usd"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if cost == 0.0 && (input > 0 || output > 0) {
        cost = otto_usage::estimate_cost(&model, input, output);
    }

    Some(UsageEvent {
        workspace_id: workspace_id.clone(),
        session_id: session_id.clone(),
        provider: provider.to_string(),
        model,
        kind: kind.to_string(),
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        cost_usd: cost,
        duration_ms: 0,
    })
}
