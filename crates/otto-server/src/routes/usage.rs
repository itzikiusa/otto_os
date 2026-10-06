//! Usage & metrics endpoints, backed by the embedded ClickHouse engine.
//!
//! Reads (`status` / `summary` / `by-kind` / `report`) need `Usage:View` (the
//! policy layer checks it): root sees every session, anyone else only the
//! sessions they created (`scope: "own"`, external sessions excluded) — so a
//! member granted Usage no longer lands on a dead end (U3). Config, install,
//! budget writes, system metrics and the ccusage cross-check stay root-only. The
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
    AttributionDimension, AttributionRow, CcusageCheck, CcusageCheckReq, FeatureUsage, ForecastReq,
    ForecastResp, MetricPoint, SessionTotals, UsageConfig, UsageEvent, UsageReport, UsageScope,
    UsageStatus, UsageSummary,
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
    let value = serde_json::to_value(cfg).map_err(|e| {
        ApiError(otto_core::Error::Internal(format!(
            "serialize usage config: {e}"
        )))
    })?;
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

/// `GET /usage/status` — engine + ClickHouse health. Non-root callers get
/// the health bits only (paths, version, sizes and row counts redacted).
pub async fn status(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<UsageStatus>> {
    let mut st = ctx.usage.status().await;
    if !user.is_root {
        st.binary = None;
        st.version = None;
        st.data_dir = String::new();
        st.usage_rows = 0;
        st.metric_rows = 0;
        st.disk_bytes = 0;
    }
    Ok(Json(st))
}

/// How long the session-label projection is reused across usage requests
/// (summary, by-kind and report open together; budgets poll).
const USAGE_LABELS_TTL: std::time::Duration = std::time::Duration::from_secs(5);

type UsageLabels = std::sync::Arc<Vec<otto_state::UsageLabelRow>>;

static USAGE_LABELS_CACHE: std::sync::Mutex<Option<(std::time::Instant, UsageLabels)>> =
    std::sync::Mutex::new(None);

/// Every session's label inputs, read once per request (and memoised for
/// USAGE_LABELS_TTL): feeds the enrichment, the per-kind rollup and
/// (non-root) the caller's own-session scope. A narrow projection — it used
/// to decode every full session row (meta JSON included) to label ≤50 (R4).
async fn all_sessions(ctx: &ServerCtx) -> UsageLabels {
    if let Some((at, rows)) = USAGE_LABELS_CACHE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
    {
        if at.elapsed() < USAGE_LABELS_TTL {
            return std::sync::Arc::clone(rows);
        }
    }
    match otto_state::SessionsRepo::new(ctx.pool.clone())
        .list_usage_labels()
        .await
    {
        Ok(rows) => {
            let rows = std::sync::Arc::new(rows);
            *USAGE_LABELS_CACHE.lock().unwrap_or_else(|p| p.into_inner()) =
                Some((std::time::Instant::now(), std::sync::Arc::clone(&rows)));
            rows
        }
        Err(e) => {
            tracing::warn!("usage: could not list sessions for enrichment: {e}");
            std::sync::Arc::new(Vec::new())
        }
    }
}

/// Session ids a non-root caller may see (the ones they created); `None` for
/// root (everything).
fn own_session_ids(
    user: &otto_core::domain::User,
    all: &[otto_state::UsageLabelRow],
) -> Option<Vec<String>> {
    own_ids(
        user.is_root,
        &user.id,
        all.iter().map(|s| (s.id.as_str(), s.created_by.as_str())),
    )
}

/// [`own_session_ids`] over `(session_id, created_by)` pairs.
fn own_ids<'a>(
    is_root: bool,
    user_id: &str,
    sessions: impl Iterator<Item = (&'a str, &'a str)>,
) -> Option<Vec<String>> {
    if is_root {
        return None;
    }
    Some(
        sessions
            .filter(|(_, by)| *by == user_id)
            .map(|(id, _)| id.to_string())
            .collect(),
    )
}

/// `GET /usage/summary?days=N` — provider/day/session/model/feature rollups.
/// Root: every session; others: their own sessions (`scope: "own"`).
pub async fn summary(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<WindowDays>,
) -> ApiResult<Json<UsageSummary>> {
    let days = q.days.unwrap_or(30).clamp(1, 3650);
    // ONE unfiltered sessions scan feeds the enrichment, the per-kind rollup
    // and the scope (it used to run twice per summary — perf O6). Root needs
    // no scope from it, so the label read and both ClickHouse scans (the
    // grouped one + the per-session totals the by-kind rollup reuses) run
    // CONCURRENTLY (R4); a non-root scope must know its own ids first.
    let (all_sessions, own, otto_only, summary) = if user.is_root {
        let otto_only = q.otto_only.unwrap_or(true);
        let (all, summary) = tokio::join!(
            all_sessions(&ctx),
            ctx.usage.summary_scoped(days, otto_only, UsageScope::All)
        );
        (all, None, otto_only, summary)
    } else {
        let all = all_sessions(&ctx).await;
        let own = own_session_ids(&user, &all);
        // A non-root view never includes external (machine-wide) sessions.
        let otto_only = true;
        let scope = UsageScope::Sessions(own.as_deref().unwrap_or_default());
        let summary = ctx.usage.summary_scoped(days, otto_only, scope).await;
        (all, own, otto_only, summary)
    };
    let mut summary = summary.map_err(ApiError)?;
    enrich_sessions(&ctx, &mut summary.sessions, &all_sessions).await;
    summary.by_kind =
        by_kind_rollup_with(&ctx, days, otto_only, &all_sessions, own.as_deref()).await;
    Ok(Json(summary))
}

#[derive(Debug, Deserialize)]
pub struct ReportQuery {
    pub days: Option<u32>,
    pub otto_only: Option<bool>,
    /// Session leaderboard cap (default 100, max 1000 — the export's).
    pub sessions_limit: Option<u32>,
    /// Comma list of opt-in tables; `daily_models` (the export's day×model
    /// table) is the only one.
    pub include: Option<String>,
}

/// Default report session cap: the page renders 100 at a time.
const REPORT_DEFAULT_SESSIONS: u32 = 100;

/// `GET /usage/report?days=N&otto_only=B&sessions_limit=N&include=daily_models`
/// — the ccusage-style report (daily / monthly / model / session tables).
/// Scoped like the summary. Slim by default (100 sessions, no day×model).
pub async fn report(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<ReportQuery>,
) -> ApiResult<Json<UsageReport>> {
    let days = q.days.unwrap_or(30).clamp(1, 3650);
    let opts = otto_usage::ReportOptions {
        sessions_limit: q
            .sessions_limit
            .unwrap_or(REPORT_DEFAULT_SESSIONS)
            .clamp(1, otto_usage::REPORT_SESSION_LIMIT),
        daily_models: q
            .include
            .as_deref()
            .is_some_and(|i| i.split(',').any(|t| t.trim() == "daily_models")),
    };
    let all_sessions = all_sessions(&ctx).await;
    let own = own_session_ids(&user, &all_sessions);
    let otto_only = own.is_some() || q.otto_only.unwrap_or(true);
    let scope = match &own {
        Some(ids) => UsageScope::Sessions(ids),
        None => UsageScope::All,
    };
    let mut report = ctx
        .usage
        .report_with(days, otto_only, scope, opts)
        .await
        .map_err(ApiError)?;
    enrich_sessions(&ctx, &mut report.sessions, &all_sessions).await;
    Ok(Json(report))
}

/// `POST /usage/ccusage-check {days?}` — opt-in cross-check against
/// `npx ccusage` (root: it runs a local subprocess and reads every
/// transcript on the machine). Problems come back as `ran: false` + `error`.
pub async fn ccusage_check(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<CcusageCheckReq>>,
) -> ApiResult<Json<CcusageCheck>> {
    require_root(&user)?;
    let days = body.and_then(|Json(b)| b.days).unwrap_or(7);
    Ok(Json(ctx.usage.ccusage_check(days).await))
}

/// `GET /usage/by-kind?days=N` — per-feature (review / product / channel /
/// agent / …) token + cost rollup over the window (scoped like the summary). Same classification
/// as the top-session `kind` badge; pricing is reused untouched.
pub async fn by_kind(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<WindowDays>,
) -> ApiResult<Json<Vec<FeatureUsage>>> {
    let days = q.days.unwrap_or(30).clamp(1, 3650);
    let all_sessions = all_sessions(&ctx).await;
    let own = own_session_ids(&user, &all_sessions);
    let otto_only = own.is_some() || q.otto_only.unwrap_or(true);
    Ok(Json(
        by_kind_rollup_with(&ctx, days, otto_only, &all_sessions, own.as_deref()).await,
    ))
}

/// Build the per-feature rollup over an already-loaded session list: pull
/// every session's raw token/cost sums from ClickHouse, classify each session
/// via its SQLite metadata (the same label as the session-row `kind` badge),
/// and fold into feature buckets. Best-effort — returns empty on any engine
/// error so the summary still renders.
async fn by_kind_rollup_with(
    ctx: &ServerCtx,
    days: u32,
    otto_only: bool,
    all_sessions: &[otto_state::UsageLabelRow],
    // `Some(ids)`: only these sessions count (a non-root caller's own).
    own: Option<&[String]>,
) -> Vec<FeatureUsage> {
    const SKIP: &str = "\u{0}skip";
    let own: Option<std::collections::HashSet<&str>> =
        own.map(|ids| ids.iter().map(String::as_str).collect());
    // Resolve feature labels in one SQLite scan (list_all) instead of one GET
    // per session (the original N+1). Sessions absent from the map fall back
    // to "external". `feature_usage` issues its own `session_totals` query
    // internally, so we skip the pre-check and go straight to building the map.
    let labels: std::collections::HashMap<String, String> = all_sessions
        .iter()
        .map(|s| (s.id.clone(), session_kind_label(s)))
        .collect();
    ctx.usage
        .feature_usage(days, otto_only, |t: &SessionTotals| {
            if own
                .as_ref()
                .is_some_and(|o| !o.contains(t.session_id.as_str()))
            {
                return SKIP.to_string();
            }
            labels
                .get(&t.session_id)
                .cloned()
                .unwrap_or_else(|| "external".to_string())
        })
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f.feature != SKIP)
        .collect()
}

/// Enrich top-session rows with the Otto session title (pane name), kind
/// (review / product / channel / agent…), and workspace name — all looked up
/// from SQLite in one pass rather than N individual GETs.
///
/// Strategy: `list_all` returns every session (admin read); we filter by the
/// set of ids we need, then walk the result building the same enrichment the
/// old N-sequential-get path did. For the typical top-50 session leaderboard
/// this cuts N round-trips to a single SQLite scan.
async fn enrich_sessions(
    ctx: &ServerCtx,
    sessions: &mut [otto_usage::SessionUsage],
    // The unfiltered cross-workspace label read; rows are already scoped by
    // the route, so no ownership narrowing is needed here.
    all_sessions: &[otto_state::UsageLabelRow],
) {
    if sessions.is_empty() {
        return;
    }

    let needed_ids: std::collections::HashSet<String> =
        sessions.iter().map(|s| s.session_id.clone()).collect();

    let sess_map: std::collections::HashMap<String, &otto_state::UsageLabelRow> = all_sessions
        .iter()
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
fn session_kind_label(s: &otto_state::UsageLabelRow) -> String {
    if let Some(src) = s.source.as_deref() {
        return match src {
            "product-analysis" | "product-plan" => "product".to_string(),
            other => other.to_string(), // "review", "channel"
        };
    }
    // `sessions.kind` is the lowercase SessionKind (`agent` / `connection`).
    s.kind.clone()
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

/// Crate-public accessor for the budget status computation (a foreground
/// read: Mission Control's budget card).
pub(crate) async fn budget_status_pub(
    ctx: &ServerCtx,
    cfg: UsageBudgetConfig,
) -> otto_core::api::UsageBudgetStatus {
    budget_status(ctx, cfg).await
}

/// [`budget_status`] for BACKGROUND checks — the budget sampler in
/// `monitor.rs` and [`check_budget`] (S9-303). Reads spend without resetting
/// ClickHouse's idle clock and without waking a parked server (it serves the
/// last spend it saw); `None` when there is no spend to judge yet.
pub(crate) async fn budget_status_background(
    ctx: &ServerCtx,
    mut cfg: UsageBudgetConfig,
) -> Option<UsageBudgetStatus> {
    let window_days = budget_window(&cfg);
    cfg.window_days = window_days;
    let totals = ctx
        .usage
        .session_totals_background(window_days, true)
        .await?;
    Some(fold_budget_status(ctx, cfg, &totals).await)
}

/// The configured budget window in days (0 = the 30-day default).
fn budget_window(cfg: &UsageBudgetConfig) -> u32 {
    if cfg.window_days == 0 {
        30
    } else {
        cfg.window_days.clamp(1, 3650)
    }
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
    let value = serde_json::to_value(&cfg).map_err(|e| {
        ApiError(otto_core::Error::Internal(format!(
            "serialize budgets: {e}"
        )))
    })?;
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
    let window_days = budget_window(&cfg);
    cfg.window_days = window_days;
    let totals = ctx
        .usage
        .session_totals(window_days, true)
        .await
        .unwrap_or_default();
    fold_budget_status(ctx, cfg, &totals).await
}

/// Fold per-session totals into spend-vs-cap rows (`cfg.window_days` already
/// normalised).
async fn fold_budget_status(
    ctx: &ServerCtx,
    cfg: UsageBudgetConfig,
    totals: &[otto_usage::SessionTotals],
) -> UsageBudgetStatus {
    let window_days = cfg.window_days;
    // One pass over per-session totals folds spend into both buckets.
    let mut by_ws: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut by_provider: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for t in totals {
        *by_ws.entry(t.workspace_id.clone()).or_default() += t.cost_usd;
        *by_provider.entry(t.provider.clone()).or_default() += t.cost_usd;
    }

    let mut rows = Vec::new();
    for b in &cfg.workspaces {
        if b.monthly_usd <= 0.0 {
            continue;
        }
        let spent = by_ws.get(&b.workspace_id).copied().unwrap_or(0.0);
        let label = ctx
            .workspaces
            .get(&b.workspace_id)
            .await
            .ok()
            .map(|w| w.name);
        rows.push(make_row(
            "workspace",
            &b.workspace_id,
            label,
            b.monthly_usd,
            spent,
        ));
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

fn make_row(
    scope: &str,
    key: &str,
    label: Option<String>,
    limit: f64,
    spent: f64,
) -> BudgetStatusRow {
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
    // A background read (S9-303): a gate consulted per swarm turn / workflow
    // step must not keep ClickHouse awake. Only when there is no spend to
    // judge at all (parked before any scan) does it fall back to a foreground
    // read — failing open on a cap could let a blocked run through.
    let status = match budget_status_background(ctx, cfg.clone()).await {
        Some(status) => status,
        None => budget_status(ctx, cfg.clone()).await,
    };
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

    // Origin falls back to "ingest" so rows recorded through this endpoint are
    // never mistaken for transcript-tailer rows (which carry no dims at all —
    // the tailer's one-time dedup rebuild purges exactly that shape).
    let origin = {
        let o = dim_val("origin");
        if o.is_empty() {
            "ingest".to_string()
        } else {
            o
        }
    };
    ctx.usage.record(UsageEvent {
        ts: None,
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
        origin,
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
        obj.and_then(|o| o.get(key))
            .and_then(Value::as_u64)
            .unwrap_or(0)
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
        ts: None,
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

// ---------------------------------------------------------------------------
// Per-session tokens + cost (review A5)
// ---------------------------------------------------------------------------

/// `GET /sessions/{id}/usage` — this session's token + cost totals over its
/// whole recorded history (`null` when nothing was recorded or usage tracking
/// is unavailable — never a misleading 0). Session viewer (owner or workspace
/// admin, the transcript gate) + `Usage:View` (policy).
pub async fn session_usage(
    axum::extract::Path(id): axum::extract::Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Option<SessionTotals>>> {
    crate::routes::transcript::session_gate(
        &ctx,
        &user,
        &id,
        otto_core::domain::WorkspaceRole::Viewer,
    )
    .await?;
    if !ctx.usage.available() {
        return Ok(Json(None));
    }
    Ok(Json(ctx.usage.session_totals_for(&id, None).await))
}

#[derive(Debug, Deserialize)]
pub struct SessionsUsageQuery {
    /// Days of history to roll up (default 30, 1–365).
    pub days: Option<u32>,
}

/// `GET /workspaces/{wid}/sessions/usage` response.
#[derive(Debug, serde::Serialize)]
pub struct WorkspaceSessionsUsage {
    /// False when the usage engine is unavailable / disabled — the UI then
    /// hides every tokens/cost affordance instead of showing zeros.
    pub available: bool,
    pub days: u32,
    /// One row per session of this workspace the caller may see that has
    /// recorded usage in the window (sessions without usage are absent).
    pub sessions: Vec<SessionTotals>,
}

/// How long the daemon-wide per-session rollup is reused. Every rollup spawns
/// a ClickHouse query over the whole window, and the sidebar asks per
/// workspace switch / minute; a minute of staleness is invisible there.
const SESSIONS_USAGE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

type SessionsUsageCache =
    std::sync::Mutex<Option<(std::time::Instant, u32, std::sync::Arc<Vec<SessionTotals>>)>>;

static SESSIONS_USAGE_CACHE: SessionsUsageCache = std::sync::Mutex::new(None);

/// The daemon-wide `session_totals(days)` rollup, cached for
/// [`SESSIONS_USAGE_TTL`] per window. Errors are not cached.
async fn cached_session_totals(
    ctx: &ServerCtx,
    days: u32,
) -> Result<std::sync::Arc<Vec<SessionTotals>>, ApiError> {
    if let Some((at, d, rows)) = SESSIONS_USAGE_CACHE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
    {
        if *d == days && at.elapsed() < SESSIONS_USAGE_TTL {
            return Ok(std::sync::Arc::clone(rows));
        }
    }
    let rows = std::sync::Arc::new(
        ctx.usage
            .session_totals(days, false)
            .await
            .map_err(ApiError)?,
    );
    *SESSIONS_USAGE_CACHE
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some((
        std::time::Instant::now(),
        days,
        std::sync::Arc::clone(&rows),
    ));
    Ok(rows)
}

/// Keep the rollup rows for `visible` sessions only.
fn filter_session_totals(
    rows: &[SessionTotals],
    visible: &std::collections::HashSet<&str>,
) -> Vec<SessionTotals> {
    rows.iter()
        .filter(|r| visible.contains(r.session_id.as_str()))
        .cloned()
        .collect()
}

/// `GET /workspaces/{wid}/sessions/usage?days=30` — tokens + cost per session
/// of this workspace (the Agents sidebar's tokens sort / row tooltip). Same
/// visibility as the session list: a workspace admin (or root) sees every
/// session, anyone else only their own. Backed by one cached daemon-wide
/// rollup ([`cached_session_totals`]). `Usage:View` (policy) + workspace
/// viewer (here).
pub async fn workspace_sessions_usage(
    axum::extract::Path(wid): axum::extract::Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<SessionsUsageQuery>,
) -> ApiResult<Json<WorkspaceSessionsUsage>> {
    use otto_core::domain::WorkspaceRole;
    crate::auth::require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let days = q.days.unwrap_or(30).clamp(1, 365);
    if !ctx.usage.available() {
        return Ok(Json(WorkspaceSessionsUsage {
            available: false,
            days,
            sessions: Vec::new(),
        }));
    }
    let repo = otto_state::SessionsRepo::new(ctx.pool.clone());
    let admin = user.is_root
        || crate::auth::require_ws_role(&ctx, &user, &wid, WorkspaceRole::Admin)
            .await
            .is_ok();
    // Only the ids of the sessions the sidebar shows (live, foreground +
    // channel rows): it used to decode every row of the workspace — meta and
    // 1.9 k hidden review agents included — and return usage for all of them.
    let ids = repo
        .visible_ids(&wid, (!admin).then_some(&user.id))
        .await
        .map_err(ApiError)?;
    let visible: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
    let rows = cached_session_totals(&ctx, days).await?;
    Ok(Json(WorkspaceSessionsUsage {
        available: true,
        days,
        sessions: filter_session_totals(&rows, &visible),
    }))
}

#[cfg(test)]
mod session_usage_tests {
    use super::*;

    fn row(id: &str, tokens: u64) -> SessionTotals {
        SessionTotals {
            session_id: id.into(),
            workspace_id: "w".into(),
            provider: "claude".into(),
            events: 1,
            input_tokens: tokens,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: tokens,
            cost_usd: 0.5,
            ..Default::default()
        }
    }

    #[test]
    fn filter_keeps_only_visible_sessions() {
        let rows = vec![row("a", 10), row("b", 20), row("c", 30)];
        let visible: std::collections::HashSet<&str> = ["a", "c", "zzz"].into_iter().collect();
        let got = filter_session_totals(&rows, &visible);
        let ids: Vec<&str> = got.iter().map(|r| r.session_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c"]);
    }
}

#[cfg(test)]
mod scope_tests {
    use super::own_ids;

    #[test]
    fn root_sees_everything_others_only_their_sessions() {
        let rows = [("s1", "alice"), ("s2", "bob"), ("s3", "alice")];
        assert_eq!(own_ids(true, "alice", rows.iter().copied()), None);
        assert_eq!(
            own_ids(false, "alice", rows.iter().copied()),
            Some(vec!["s1".to_string(), "s3".to_string()])
        );
        assert_eq!(own_ids(false, "carol", rows.iter().copied()), Some(vec![]));
    }
}
