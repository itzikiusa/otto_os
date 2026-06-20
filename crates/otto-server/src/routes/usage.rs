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
use otto_core::api::{BudgetStatusRow, UsageBudgetConfig, UsageBudgetStatus};
use otto_core::workref::WorkRef;
use otto_core::Id;
use otto_state::SettingsRepo;
use otto_usage::{
    AttributionDimension, AttributionRow, FeatureUsage, ForecastReq, ForecastResp, MetricPoint,
    SessionTotals, UsageConfig, UsageEvent, UsageStatus, UsageSummary,
};
use serde::Deserialize;
use serde_json::Value;

use crate::auth::{require_root, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// `settings` key the usage config is persisted under.
const SETTINGS_KEY: &str = "usage";
/// `settings` key the usage-budget config is persisted under.
const BUDGETS_KEY: &str = "usage_budgets";

async fn load_config(ctx: &ServerCtx) -> UsageConfig {
    SettingsRepo::new(ctx.pool.clone())
        .get(SETTINGS_KEY)
        .await
        .ok()
        .flatten()
        .as_ref()
        .map(|v| UsageConfig::from_json(Some(v)))
        .unwrap_or_default()
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

/// `GET /usage/summary?days=N` — provider/day/session/feature rollups (root).
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
    summary.by_kind = by_kind_rollup(&ctx, days, otto_only).await;
    Ok(Json(summary))
}

/// `GET /usage/by-kind?days=N` — per-feature (review / product / channel /
/// agent / …) token + cost rollup over the window (root). Same classification
/// as the top-session `kind` badge; pricing is reused untouched.
pub async fn by_kind(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<WindowDays>,
) -> ApiResult<Json<Vec<FeatureUsage>>> {
    require_root(&user)?;
    let days = q.days.unwrap_or(30).clamp(1, 3650);
    let otto_only = q.otto_only.unwrap_or(true);
    Ok(Json(by_kind_rollup(&ctx, days, otto_only).await))
}

/// Build the per-feature rollup: pull every session's raw token/cost sums from
/// ClickHouse, classify each session via its SQLite metadata (the same label as
/// the session-row `kind` badge), and fold into feature buckets. Best-effort —
/// returns empty on any engine error so the summary still renders.
async fn by_kind_rollup(ctx: &ServerCtx, days: u32, otto_only: bool) -> Vec<FeatureUsage> {
    // Resolve feature labels in one SQLite scan (list_all) instead of one GET
    // per session (the original N+1). Sessions absent from the map fall back
    // to "external". `feature_usage` issues its own `session_totals` query
    // internally, so we skip the pre-check and go straight to building the map.
    let repo = otto_state::SessionsRepo::new(ctx.pool.clone());
    let all_sessions = repo.list_all().await.unwrap_or_default();
    let labels: std::collections::HashMap<String, String> = all_sessions
        .into_iter()
        .map(|s| (s.id.clone(), session_kind_label(&s)))
        .collect();
    ctx.usage
        .feature_usage(days, otto_only, |t: &SessionTotals| {
            labels
                .get(&t.session_id)
                .cloned()
                .unwrap_or_else(|| "external".to_string())
        })
        .await
        .unwrap_or_default()
}

/// Enrich top-session rows with the Otto session title (pane name), kind
/// (review / product / channel / agent…), and workspace name — all looked up
/// from SQLite in one pass rather than N individual GETs.
///
/// Strategy: `list_all` returns every session (admin read); we filter by the
/// set of ids we need, then walk the result building the same enrichment the
/// old N-sequential-get path did. For the typical top-50 session leaderboard
/// this cuts N round-trips to a single SQLite scan.
async fn enrich_sessions(ctx: &ServerCtx, sessions: &mut [otto_usage::SessionUsage]) {
    if sessions.is_empty() {
        return;
    }

    let needed_ids: std::collections::HashSet<String> =
        sessions.iter().map(|s| s.session_id.clone()).collect();

    let repo = otto_state::SessionsRepo::new(ctx.pool.clone());
    // list_all is the unfiltered cross-workspace read; it's root-only at the
    // route, so no ownership narrowing is needed here.
    let all_sessions = match repo.list_all().await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("usage: could not list sessions for enrichment: {e}");
            return;
        }
    };
    let sess_map: std::collections::HashMap<String, otto_core::domain::Session> = all_sessions
        .into_iter()
        .filter(|s| needed_ids.contains(&s.id))
        .map(|s| (s.id.clone(), s))
        .collect();

    // Resolve workspace names in a second pass (one GET per unique workspace;
    // most top-50 sets share a handful of workspaces so this stays cheap).
    let mut ws_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for s in sessions.iter_mut() {
        let Some(sess) = sess_map.get(&s.session_id) else {
            continue; // external / unknown session
        };
        let title = sess.title.trim();
        if !title.is_empty() {
            s.title = Some(title.to_string());
        }
        s.kind = Some(session_kind_label(sess));
        // Mark rows whose cost was estimated via the conservative FALLBACK rate
        // card (unrecognised model). The `model` field comes from ClickHouse via
        // `any(model)`; non-empty + not-priced → "estimated".
        if !s.model.is_empty() {
            s.fallback_priced = !otto_usage::is_priced(&s.model);
        }
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

// ---------------------------------------------------------------------------
// Usage budgets (opt-in spend caps; secondary to MCP host config)
// ---------------------------------------------------------------------------

/// Crate-public accessor for the budget config; used by the budget sampler in
/// `monitor.rs` without going through the route handler.
pub(crate) async fn load_budgets_pub(ctx: &ServerCtx) -> UsageBudgetConfig {
    load_budgets(ctx).await
}

/// Crate-public accessor for the budget status computation; used by the budget
/// sampler in `monitor.rs`.
pub(crate) async fn budget_status_pub(ctx: &ServerCtx, cfg: UsageBudgetConfig) -> otto_core::api::UsageBudgetStatus {
    budget_status(ctx, cfg).await
}

/// Load the persisted budget config (defaults: enforcement off).
async fn load_budgets(ctx: &ServerCtx) -> UsageBudgetConfig {
    SettingsRepo::new(ctx.pool.clone())
        .get(BUDGETS_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// `GET /usage/budgets` — the budget config plus live status rows (spend vs cap)
/// over the configured window (root). Status is computed even when enforcement is
/// off, so the UI can preview caps before turning them on.
pub async fn budgets(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<UsageBudgetStatus>> {
    require_root(&user)?;
    let cfg = load_budgets(&ctx).await;
    Ok(Json(budget_status(&ctx, cfg).await))
}

/// `PUT /usage/budgets` — replace + persist the budget config (root). Returns the
/// new config with refreshed status. Enforcement is whatever the body sets;
/// nothing is turned on implicitly.
pub async fn put_budgets(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(mut cfg): Json<UsageBudgetConfig>,
) -> ApiResult<Json<UsageBudgetStatus>> {
    require_root(&user)?;
    cfg.window_days = if cfg.window_days == 0 {
        30
    } else {
        cfg.window_days.clamp(1, 3650)
    };
    let value = serde_json::to_value(&cfg)
        .map_err(|e| ApiError(otto_core::Error::Internal(format!("serialize budgets: {e}"))))?;
    SettingsRepo::new(ctx.pool.clone())
        .put(BUDGETS_KEY, &value)
        .await
        .map_err(ApiError)?;
    Ok(Json(budget_status(&ctx, cfg).await))
}

/// 80% of a cap is the "warning" line.
const BUDGET_WARN_FRACTION: f64 = 0.8;

/// Compute spend vs. cap for every configured budget over the window. Best-effort
/// — on any engine error spend reads as `0` (so the UI still renders the caps).
async fn budget_status(ctx: &ServerCtx, mut cfg: UsageBudgetConfig) -> UsageBudgetStatus {
    let window_days = if cfg.window_days == 0 {
        30
    } else {
        cfg.window_days.clamp(1, 3650)
    };
    cfg.window_days = window_days;

    // One pass over per-session totals folds spend into both buckets.
    let totals = ctx
        .usage
        .session_totals(window_days, true)
        .await
        .unwrap_or_default();
    let mut by_ws: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut by_provider: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for t in &totals {
        *by_ws.entry(t.workspace_id.clone()).or_default() += t.cost_usd;
        *by_provider.entry(t.provider.clone()).or_default() += t.cost_usd;
    }

    let mut rows = Vec::new();
    for b in &cfg.workspaces {
        if b.monthly_usd <= 0.0 {
            continue;
        }
        let spent = by_ws.get(&b.workspace_id).copied().unwrap_or(0.0);
        let label = ctx.workspaces.get(&b.workspace_id).await.ok().map(|w| w.name);
        rows.push(make_row("workspace", &b.workspace_id, label, b.monthly_usd, spent));
    }
    for b in &cfg.providers {
        if b.monthly_usd <= 0.0 {
            continue;
        }
        let spent = by_provider.get(&b.provider).copied().unwrap_or(0.0);
        rows.push(make_row(
            "provider",
            &b.provider,
            Some(b.provider.clone()),
            b.monthly_usd,
            spent,
        ));
    }

    UsageBudgetStatus {
        config: cfg,
        window_days,
        rows,
    }
}

fn make_row(scope: &str, key: &str, label: Option<String>, limit: f64, spent: f64) -> BudgetStatusRow {
    let used = if limit > 0.0 { spent / limit } else { 0.0 };
    BudgetStatusRow {
        scope: scope.to_string(),
        key: key.to_string(),
        label,
        limit_usd: limit,
        spent_usd: spent,
        used_fraction: used,
        warning: used >= BUDGET_WARN_FRACTION,
        exceeded: used >= 1.0,
    }
}

/// Outcome of a daemon-side budget consultation for a workspace + provider.
#[derive(Debug, Clone, Default)]
pub struct BudgetVerdict {
    /// True when enforcement is on AND `block_on_exceed` AND a relevant cap is
    /// exceeded. Callers that want to gate work check this.
    pub blocked: bool,
    /// True when enforcement is on and a relevant cap is exceeded (whether or not
    /// blocking is enabled). Callers can use this to warn prominently.
    pub exceeded: bool,
    /// Human-readable reason when exceeded/blocked (which cap, spend vs limit).
    pub reason: Option<String>,
}

/// Daemon-consultable budget check for a `(workspace, provider)`. Returns a
/// no-op verdict when enforcement is off (the default), so callers can wire this
/// in safely without changing behaviour until a root user opts in. Best-effort:
/// any engine error reads as "not exceeded".
pub async fn check_budget(ctx: &ServerCtx, workspace_id: &str, provider: &str) -> BudgetVerdict {
    let cfg = load_budgets(ctx).await;
    if !cfg.enforce {
        return BudgetVerdict::default();
    }
    let status = budget_status(ctx, cfg.clone()).await;
    for row in &status.rows {
        let relevant = (row.scope == "workspace" && row.key == workspace_id)
            || (row.scope == "provider" && row.key == provider);
        if relevant && row.exceeded {
            let reason = format!(
                "{} budget exceeded: ${:.2} spent of ${:.2} cap over {}d",
                row.scope, row.spent_usd, row.limit_usd, status.window_days
            );
            return BudgetVerdict {
                blocked: cfg.block_on_exceed,
                exceeded: true,
                reason: Some(reason),
            };
        }
    }
    BudgetVerdict::default()
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
        otto_usage::estimate_cost(
            &req.model,
            req.input_tokens,
            req.output_tokens,
            req.cache_read_tokens,
            req.cache_write_tokens,
        )
    };

    // Resolve work-graph attribution from the session's meta_json["work"]. If
    // absent or malformed the WorkRef defaults to all-None (no dims written).
    let work_ref: WorkRef = session
        .meta
        .get("work")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let dims = work_ref.dimensions();
    let dim_val = |key: &str| -> String {
        dims.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
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
        // Work-graph dims (B1) — empty strings are omitted in JSON serialization
        // via `skip_serializing_if = "String::is_empty"` on each field.
        repo_id: dim_val("repo_id"),
        branch: dim_val("branch"),
        pr_number: dim_val("pr_number"),
        story_id: dim_val("story_id"),
        swarm_task_id: dim_val("swarm_task_id"),
        workflow_id: dim_val("workflow_id"),
        channel: dim_val("channel"),
        review_id: dim_val("review_id"),
        origin: dim_val("origin"),
    });
    StatusCode::NO_CONTENT
}

// ---------------------------------------------------------------------------
// Work-graph attribution + cost forecast (B1)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AttributionQuery {
    /// Dimension to group by: repo|branch|pr|story|swarm_task|workflow|channel|review|origin
    pub by: Option<String>,
    /// Look-back window in days (default 30).
    pub days: Option<u32>,
}

/// `GET /usage/attribution?by=<dim>&days=N` — grouped cost/tokens by one
/// work-graph dimension (root). Filters empty-string keys (un-attributed rows).
pub async fn attribution(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<AttributionQuery>,
) -> ApiResult<Json<Vec<AttributionRow>>> {
    require_root(&user)?;
    let by_str = q.by.as_deref().unwrap_or("origin");
    let dim = AttributionDimension::from_str(by_str).unwrap_or(AttributionDimension::Origin);
    let days = q.days.unwrap_or(30).clamp(1, 3650);
    let rows = ctx.usage.attribution(&dim, days).await.map_err(ApiError)?;
    Ok(Json(rows))
}

/// `POST /usage/forecast` — estimate cost before a run (root). Body:
/// `{feature, provider, est_tokens?}`. Returns `{projected_cost_usd, basis}`.
pub async fn forecast(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ForecastReq>,
) -> ApiResult<Json<ForecastResp>> {
    require_root(&user)?;
    Ok(Json(ctx.usage.forecast(&req).await))
}

/// Map a session's activity-trail entry to a usage row, mining token counts /
/// model / cost from the entry's `detail` payload when present. Returns `None`
/// for noise (task-tracker churn, human notes). This is the automatic path —
/// every meaningful agent action becomes a usage event (with tokens when the
/// provider reports them, otherwise an activity count).
///
/// The optional `work` reference is flattened into the work-graph attribution
/// columns (B1). Pass `None` or `Some(&WorkRef::default())` for sessions that
/// have no work attribution.
pub fn trail_to_usage(
    workspace_id: &Id,
    session_id: &Id,
    provider: &str,
    event: &otto_core::domain::TrailEvent,
) -> Option<UsageEvent> {
    trail_to_usage_with_work(workspace_id, session_id, provider, event, None)
}

/// Like [`trail_to_usage`] but stamps work-graph dims from `work` when provided.
/// Callers that have the session's `WorkRef` in scope should prefer this.
pub fn trail_to_usage_with_work(
    workspace_id: &Id,
    session_id: &Id,
    provider: &str,
    event: &otto_core::domain::TrailEvent,
    work: Option<&WorkRef>,
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
    if cost == 0.0 && (input > 0 || output > 0 || cache_read > 0 || cache_write > 0) {
        cost = otto_usage::estimate_cost(&model, input, output, cache_read, cache_write);
    }

    // Flatten work-ref dims (empty when no work ref supplied).
    let empty = WorkRef::default();
    let wr = work.unwrap_or(&empty);
    let dims = wr.dimensions();
    let dim_val = |key: &str| -> String {
        dims.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };

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
        repo_id: dim_val("repo_id"),
        branch: dim_val("branch"),
        pr_number: dim_val("pr_number"),
        story_id: dim_val("story_id"),
        swarm_task_id: dim_val("swarm_task_id"),
        workflow_id: dim_val("workflow_id"),
        channel: dim_val("channel"),
        review_id: dim_val("review_id"),
        origin: dim_val("origin"),
    })
}
