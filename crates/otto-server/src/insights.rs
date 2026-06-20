//! Otto Insights — Phase 2: an opt-in, catch-up scheduler that runs the
//! `insights` skill on a cadence, plus the reports / config / run HTTP API.
//!
//! Insights are **global** (not per-workspace). The user opts in per cadence
//! (`daily` / `weekly` / `monthly`, all default OFF). A background supervisor
//! ticks ~hourly and, for each ENABLED cadence, computes the currently-due
//! period (previous calendar day / ISO week / month) and runs the skill iff that
//! period has no report yet (idempotency = the `insights` history index/files
//! the skill writes). One run at a time, bounded catch-up (only the most-recent
//! missed period per cadence).
//!
//! A "run" = spawning a real, headless agent session that executes the
//! `insights` skill for that period (mirroring how `product_run::run_lens_session`
//! runs a product lens). The session runs on the global default provider in a
//! neutral cwd (the Otto data dir). If the `insights` skill is not installed in
//! the Library, the run is SKIPPED and a warning logged (it's a manual-install
//! skill; the UI tells the user to install it).
//!
//! Data dir layout the skill writes (mirrored here for parsing / idempotency):
//! ```text
//! <data_dir>/insights/
//!   config.json                                  ← THIS module's opt-in config
//!   index.json                                   ← rolling series + action ledger
//!   <kind>/                                       (daily | weekly | monthly)
//!     metrics-<kind>-<start>_<end>.json
//!     summary-<kind>-<start>_<end>.md
//!     report-<kind>-<start>_<end>.html
//! ```
//! where `<start>`/`<end>` are `YYYYMMDD`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Datelike, Days, Months, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use otto_core::event::Event;

use crate::auth::{require_root, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// Cadence kind
// ---------------------------------------------------------------------------

/// One insights cadence. The wire / skill `--period` token is `day|week|month`;
/// the on-disk directory and file prefixes use `daily|weekly|monthly`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Day,
    Week,
    Month,
}

impl Kind {
    /// The three cadences, in catch-up order (cheap → expensive).
    pub const ALL: [Kind; 3] = [Kind::Day, Kind::Week, Kind::Month];

    /// The `--period` token the collector accepts (`day` / `week` / `month`).
    pub fn period(self) -> &'static str {
        match self {
            Kind::Day => "day",
            Kind::Week => "week",
            Kind::Month => "month",
        }
    }

    /// The on-disk subdir + file-prefix word (`daily` / `weekly` / `monthly`).
    pub fn word(self) -> &'static str {
        match self {
            Kind::Day => "daily",
            Kind::Week => "weekly",
            Kind::Month => "monthly",
        }
    }

    /// Parse the API `period` field (`day|week|month`). `None` for anything else.
    pub fn from_period(s: &str) -> Option<Kind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "day" | "daily" => Some(Kind::Day),
            "week" | "weekly" => Some(Kind::Week),
            "month" | "monthly" => Some(Kind::Month),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Config (opt-in per cadence) — persisted at <data_dir>/insights/config.json
// ---------------------------------------------------------------------------

/// Global opt-in config. ALL default `false` (insights are off until the user
/// turns a cadence on).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightsConfig {
    #[serde(default)]
    pub daily: bool,
    #[serde(default)]
    pub weekly: bool,
    #[serde(default)]
    pub monthly: bool,
}

impl InsightsConfig {
    /// Whether `kind` is opted in.
    pub fn enabled(&self, kind: Kind) -> bool {
        match kind {
            Kind::Day => self.daily,
            Kind::Week => self.weekly,
            Kind::Month => self.monthly,
        }
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Resolve `<data_dir>/insights` from the context library root
/// (`<data_dir>/library`). Falls back to the library root's own parent (or the
/// library root itself if it has no parent) so this never panics.
pub fn insights_dir(ctx: &ServerCtx) -> PathBuf {
    let lib = &ctx.context_library.root;
    let data_dir = lib.parent().unwrap_or(lib);
    data_dir.join("insights")
}

fn config_path(dir: &Path) -> PathBuf {
    dir.join("config.json")
}

/// Read the config from `<insights>/config.json`, defaulting (all-off) when the
/// file is absent or malformed.
pub fn read_config(dir: &Path) -> InsightsConfig {
    match std::fs::read_to_string(config_path(dir)) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => InsightsConfig::default(),
    }
}

/// Persist the config to `<insights>/config.json`, creating the dir as needed.
pub fn write_config(dir: &Path, cfg: &InsightsConfig) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(cfg).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(config_path(dir), json)
}

// ---------------------------------------------------------------------------
// Period computation + idempotency (pure, unit-testable)
// ---------------------------------------------------------------------------

/// The inclusive calendar range `[start, end]` that is "currently due" for
/// `kind` at instant `now` (UTC):
/// - Day   → the previous calendar day.
/// - Week  → the previous ISO week (Monday..=Sunday).
/// - Month → the previous calendar month.
///
/// This is the `--offset 1` period the skill would generate.
pub fn due_period(kind: Kind, now: DateTime<Utc>) -> (NaiveDate, NaiveDate) {
    let today = now.date_naive();
    match kind {
        Kind::Day => {
            let d = today.pred_opt().unwrap_or(today); // yesterday
            (d, d)
        }
        Kind::Week => {
            // Monday of THIS week, then step back 7 days for last week.
            let dow = today.weekday().num_days_from_monday(); // Mon=0
            let this_monday = today
                .checked_sub_days(Days::new(dow as u64))
                .unwrap_or(today);
            let start = this_monday
                .checked_sub_days(Days::new(7))
                .unwrap_or(this_monday);
            let end = start.checked_add_days(Days::new(6)).unwrap_or(start);
            (start, end)
        }
        Kind::Month => {
            let first_this = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
            let start = first_this.checked_sub_months(Months::new(1)).unwrap_or(first_this);
            // Last day of the previous month = day before the 1st of this month.
            let end = first_this.pred_opt().unwrap_or(start);
            (start, end)
        }
    }
}

/// `YYYYMMDD` for a date (the on-disk file-name component).
fn ymd(d: NaiveDate) -> String {
    d.format("%Y%m%d").to_string()
}

/// The `index.json` `period_key` for a period: `<word>:<start>_<end>` (e.g.
/// `weekly:20260610_20260616`), matching the collector's key scheme.
pub fn period_key(kind: Kind, start: NaiveDate, end: NaiveDate) -> String {
    format!("{}:{}_{}", kind.word(), ymd(start), ymd(end))
}

/// Whether the period `[start, end]` for `kind` already has a report.
///
/// Mirrors the skill's `already_generated`: the period is treated as **done**
/// when ANY of its expected artifacts is present — the rolling `index.json`
/// `series` has a row for this `period_key`, OR the period's `report-*.html`
/// exists, OR its `metrics-*.json` exists. (The brief says "treat presence as
/// done"; checking all three is the most permissive de-dup so the scheduler
/// never re-runs a period the skill already covered.)
pub fn period_done(dir: &Path, kind: Kind, start: NaiveDate, end: NaiveDate) -> bool {
    let key = period_key(kind, start, end);
    if index_has_period(dir, &key) {
        return true;
    }
    let kind_dir = dir.join(kind.word());
    let stem = format!("{}-{}_{}", kind.word(), ymd(start), ymd(end));
    kind_dir.join(format!("report-{stem}.html")).exists()
        || kind_dir.join(format!("metrics-{stem}.json")).exists()
}

/// Read `<insights>/index.json` and report whether its `series` array contains
/// a row whose `period_key` equals `key`.
fn index_has_period(dir: &Path, key: &str) -> bool {
    let Ok(raw) = std::fs::read_to_string(dir.join("index.json")) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    v.get("series")
        .and_then(|s| s.as_array())
        .map(|rows| {
            rows.iter()
                .any(|r| r.get("period_key").and_then(|k| k.as_str()) == Some(key))
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Report listing
// ---------------------------------------------------------------------------

/// One stored insights report, surfaced by `GET /insights/reports`.
#[derive(Debug, Clone, Serialize)]
pub struct ReportView {
    /// `daily` | `weekly` | `monthly`.
    pub kind: String,
    /// Inclusive period start (`YYYY-MM-DD`).
    pub period_start: String,
    /// Inclusive period end (`YYYY-MM-DD`).
    pub period_end: String,
    /// Absolute path of the `report-*.html`, when present.
    pub html_path: Option<String>,
    /// First ~80 lines of the `summary-*.md`, when present.
    pub summary: String,
    /// RFC3339-ish modified time of the report (or summary/metrics) file.
    pub created_at: String,
}

/// List every stored report under `<insights>/<kind>/`, newest first.
///
/// One report per period = one `summary-*.md` / `report-*.html` / `metrics-*.json`
/// triple keyed by `(kind, start, end)`. We enumerate the three subdirs and
/// build a view per discovered period, preferring the HTML mtime for `created_at`.
pub fn list_reports(dir: &Path) -> Vec<ReportView> {
    let mut out: Vec<ReportView> = Vec::new();
    for kind in Kind::ALL {
        let kind_dir = dir.join(kind.word());
        let Ok(entries) = std::fs::read_dir(&kind_dir) else {
            continue;
        };
        // Collect distinct (start,end) periods from any artifact file present.
        let mut periods: std::collections::BTreeSet<(String, String)> = Default::default();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some((start, end)) = parse_period_from_filename(&name, kind.word()) {
                periods.insert((start, end));
            }
        }
        for (start_ymd, end_ymd) in periods {
            let stem = format!("{}-{}_{}", kind.word(), start_ymd, end_ymd);
            let html = kind_dir.join(format!("report-{stem}.html"));
            let summary_md = kind_dir.join(format!("summary-{stem}.md"));
            let metrics = kind_dir.join(format!("metrics-{stem}.json"));

            let html_path = html.exists().then(|| html.to_string_lossy().into_owned());
            let summary = std::fs::read_to_string(&summary_md)
                .map(|s| truncate_lines(&s, 80))
                .unwrap_or_default();
            // created_at = mtime of the freshest existing artifact.
            let created_at = [&html, &summary_md, &metrics]
                .iter()
                .filter_map(|p| std::fs::metadata(p).ok().and_then(|m| m.modified().ok()))
                .max()
                .map(fmt_systime)
                .unwrap_or_default();

            out.push(ReportView {
                kind: kind.word().to_string(),
                period_start: dashed(&start_ymd),
                period_end: dashed(&end_ymd),
                html_path,
                summary,
                created_at,
            });
        }
    }
    // Newest first by period end, then start.
    out.sort_by(|a, b| {
        (b.period_end.as_str(), b.period_start.as_str())
            .cmp(&(a.period_end.as_str(), a.period_start.as_str()))
    });
    out
}

/// Parse `(startYMD, endYMD)` out of an insights artifact filename of the form
/// `<prefix>-<word>-<start>_<end>.<ext>` where `<word>` is the kind word. Returns
/// `None` for non-matching names. Accepts `report-`, `summary-`, `metrics-`.
fn parse_period_from_filename(name: &str, word: &str) -> Option<(String, String)> {
    let stem = name
        .strip_prefix("report-")
        .or_else(|| name.strip_prefix("summary-"))
        .or_else(|| name.strip_prefix("metrics-"))?;
    // strip the extension
    let stem = stem.rsplit_once('.').map(|(s, _)| s).unwrap_or(stem);
    // now: <word>-<start>_<end>
    let rest = stem.strip_prefix(word)?.strip_prefix('-')?;
    let (start, end) = rest.split_once('_')?;
    if start.len() == 8 && end.len() == 8 && start.chars().all(|c| c.is_ascii_digit()) {
        Some((start.to_string(), end.to_string()))
    } else {
        None
    }
}

/// `YYYYMMDD` → `YYYY-MM-DD` (best-effort; returns the input if not 8 digits).
fn dashed(ymd: &str) -> String {
    if ymd.len() == 8 {
        format!("{}-{}-{}", &ymd[0..4], &ymd[4..6], &ymd[6..8])
    } else {
        ymd.to_string()
    }
}

fn truncate_lines(s: &str, max: usize) -> String {
    s.lines().take(max).collect::<Vec<_>>().join("\n")
}

fn fmt_systime(t: std::time::SystemTime) -> String {
    DateTime::<Utc>::from(t).to_rfc3339()
}

// ---------------------------------------------------------------------------
// The run: spawn a headless session that executes the insights skill
// ---------------------------------------------------------------------------

const INSIGHTS_SKILL: &str = "insights";

/// Timeout for one insights run (the skill collects, classifies, renders HTML).
const RUN_TIMEOUT: Duration = Duration::from_secs(900);

/// Build the headless prompt that drives the `insights` skill for one period.
pub fn build_run_prompt(kind: Kind, offset: i64) -> String {
    format!(
        "Run the `insights` skill to generate the usage report for the {period} \
         period at --offset {offset} (the previous {period} when offset is 1). \
         Invoke its collector with `--period {period} --offset {offset}`, follow \
         the skill's full method (collect, classify facet-less sessions, compare \
         to the prior comparable period, render the self-contained HTML report), \
         and store all three artifacts (report HTML, summary markdown, metrics \
         JSON) plus update index.json. The report is for the period that ended; \
         do not ask the user any questions — run it end-to-end and stop when the \
         report is written. If the period was already generated, note that and stop.",
        period = kind.period(),
        offset = offset,
    )
}

/// Spawn a real, openable agent session that runs the `insights` skill for
/// `(kind, offset)`. The session uses the global default provider, runs in a
/// neutral cwd (the Otto data dir), and is NOT awaited to completion here — it
/// runs headlessly like the planner / product lens sessions.
///
/// Returns the spawned session id on success. Returns `Ok(None)` (a no-op) when
/// the `insights` skill is not installed in the Library — the caller logs a
/// warning and the UI tells the user to install it.
pub async fn run_insights(ctx: &ServerCtx, kind: Kind, offset: i64) -> otto_core::Result<Option<otto_core::Id>> {
    // The insights skill is manual-install. If absent, skip (don't spawn a
    // session that would just say "no such skill").
    let installed = ctx
        .context_library
        .skill_path(INSIGHTS_SKILL)
        .map(|p| p.exists())
        .unwrap_or(false);
    if !installed {
        warn!("insights: skill '{INSIGHTS_SKILL}' not installed in the library — skipping run; install it from Settings → Skills");
        return Ok(None);
    }

    // Pick a host workspace + actor user. Insights are global, but a session
    // row needs a valid workspace_id + created_by FK, so use the first
    // non-archived workspace and one of its members (a root user if available).
    let Some((ws, user_id)) = pick_host(ctx).await else {
        warn!("insights: no workspace/user available to host the run — skipping");
        return Ok(None);
    };

    // Neutral cwd: the Otto data dir (parent of the library root), which always
    // exists and is where the skill writes its history.
    let data_dir = insights_dir(ctx)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| ctx.context_library.root.clone());
    let cwd = data_dir.to_string_lossy().into_owned();

    let provider = default_provider(ctx).await;
    otto_sessions::trust::ensure_trusted(&provider, &cwd);

    let prompt = build_run_prompt(kind, offset);
    let req = otto_core::api::CreateSessionReq {
        kind: otto_core::domain::SessionKind::Agent,
        provider: Some(provider.clone()),
        title: Some(format!("Insights: {}", kind.word())),
        cwd: Some(cwd.clone()),
        connection_id: None,
        meta: Some(serde_json::json!({ "source": "insights" })),
    };

    let session = ctx.manager.create(&ws, &user_id, req, None).await?;
    let sid = session.id.clone();
    info!(session = %sid, kind = kind.word(), offset, provider = %provider, "insights: started run");

    // Inject the prompt once the TUI has drawn + settled, then let it run
    // headlessly (no result-file watch — the skill writes its own artifacts).
    let manager = Arc::clone(&ctx.manager);
    tokio::spawn(async move {
        if crate::review_session::wait_for_tui(&manager, &sid).await {
            let _ = manager
                .input(&sid, &crate::review_session::bracketed_paste(&prompt))
                .await;
            tokio::time::sleep(crate::review_session::PASTE_TO_ENTER).await;
            let _ = manager.input(&sid, b"\r").await;
        } else {
            warn!(session = %sid, "insights: session TUI never became ready");
        }
        // Give the run a generous window to finish, then archive the throwaway
        // session so it doesn't linger in the Agents list. Insights is a global
        // scheduled job and its report artifacts are written to disk
        // independently, so the session itself is disposable — mirrors the
        // review-session cleanup (which archives when done).
        tokio::time::sleep(RUN_TIMEOUT).await;
        let _ = manager.archive(&sid).await;
    });

    Ok(Some(session.id))
}

/// First non-archived workspace + a member user id to attribute the run to.
/// Prefers a root member; falls back to any member.
async fn pick_host(ctx: &ServerCtx) -> Option<(otto_core::domain::Workspace, otto_core::Id)> {
    let workspaces = ctx.workspaces.list_all().await.ok()?;
    let ws = workspaces.into_iter().find(|w| !w.archived)?;
    let members = ctx.workspaces.members(&ws.id).await.ok()?;
    // Prefer the workspace admin/first member as the actor.
    let user_id = members.first().map(|m| m.user_id.clone())?;
    Some((ws, user_id))
}

/// The global default provider for headless insights runs.
async fn default_provider(ctx: &ServerCtx) -> String {
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    otto_core::provider::resolve_provider(&[otto_core::provider::global_default(
        global_default.as_ref(),
    )])
}

// ---------------------------------------------------------------------------
// HTTP API (mounted under /api/v1 as a sibling router)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RunReq {
    /// `"day" | "week" | "month"`.
    pub period: String,
    /// `--offset` (previous period = 1). Defaults to 1 (the most-recent
    /// complete period), matching the scheduler.
    #[serde(default = "default_offset")]
    pub offset: i64,
}

fn default_offset() -> i64 {
    1
}

#[derive(Debug, Serialize)]
pub struct RunResp {
    pub started: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Set when `started == false` to explain why (e.g. skill not installed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Insights API routes. Paths are relative to the `/api/v1` mount; auth is
/// applied by the host middleware (writes additionally require root).
pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/insights/config", get(get_config).put(put_config))
        .route("/insights/reports", get(get_reports))
        .route("/insights/report", get(get_report))
        .route("/insights/run", post(post_run))
}

async fn get_config(State(ctx): State<ServerCtx>) -> ApiResult<Json<InsightsConfig>> {
    let dir = insights_dir(&ctx);
    Ok(Json(read_config(&dir)))
}

async fn put_config(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(cfg): Json<InsightsConfig>,
) -> ApiResult<Json<InsightsConfig>> {
    require_root(&user)?;
    let dir = insights_dir(&ctx);
    write_config(&dir, &cfg)
        .map_err(|e| ApiError(otto_core::Error::Internal(format!("write insights config: {e}"))))?;
    Ok(Json(cfg))
}

async fn get_reports(State(ctx): State<ServerCtx>) -> ApiResult<Json<Vec<ReportView>>> {
    let dir = insights_dir(&ctx);
    Ok(Json(list_reports(&dir)))
}

/// Query for serving a single report's HTML (`?path=<absolute html_path>`).
#[derive(Deserialize)]
struct ReportQuery {
    path: String,
}

/// Serve a report's HTML by absolute path, gated to the insights dir so an
/// authed caller can never read arbitrary files off disk. The UI loads this
/// (with the bearer token) into the report iframe.
async fn get_report(
    State(ctx): State<ServerCtx>,
    Query(q): Query<ReportQuery>,
) -> ApiResult<Html<String>> {
    let base = std::fs::canonicalize(insights_dir(&ctx))
        .map_err(|e| ApiError(otto_core::Error::Internal(format!("insights dir: {e}"))))?;
    let req = std::fs::canonicalize(Path::new(&q.path))
        .map_err(|_| ApiError(otto_core::Error::NotFound("report".into())))?;
    if !req.starts_with(&base) {
        return Err(ApiError(otto_core::Error::Forbidden(
            "path is outside the insights directory".into(),
        )));
    }
    let html = std::fs::read_to_string(&req)
        .map_err(|_| ApiError(otto_core::Error::NotFound("report".into())))?;
    Ok(Html(html))
}

async fn post_run(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RunReq>,
) -> ApiResult<Json<RunResp>> {
    require_root(&user)?;
    let kind = Kind::from_period(&req.period).ok_or_else(|| {
        ApiError(otto_core::Error::Invalid(format!(
            "period must be day|week|month, got '{}'",
            req.period
        )))
    })?;
    let offset = req.offset.max(0);

    match run_insights(&ctx, kind, offset).await {
        Ok(Some(id)) => Ok(Json(RunResp {
            started: true,
            run_id: Some(id.to_string()),
            reason: None,
        })),
        Ok(None) => Ok(Json(RunResp {
            started: false,
            run_id: None,
            reason: Some(format!(
                "the '{INSIGHTS_SKILL}' skill is not installed, or no workspace is available to host the run"
            )),
        })),
        Err(e) => Err(ApiError(e)),
    }
}

// ---------------------------------------------------------------------------
// Scheduler — the hourly-gated catch-up supervisor
// ---------------------------------------------------------------------------

/// Tick cadence (mirrors otto-improve's 60s tick); an internal hourly gate makes
/// the actual due-check run at most once per hour.
const SCAN_INTERVAL: Duration = Duration::from_secs(60);
const HOURLY_GATE: Duration = Duration::from_secs(60 * 60);

/// Handle that cancels the supervisor on drop.
pub struct InsightsSchedulerHandle {
    cancel: Arc<AtomicBool>,
    _supervisor: JoinHandle<()>,
}

impl InsightsSchedulerHandle {
    pub fn shutdown(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for InsightsSchedulerHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The opt-in, catch-up insights scheduler. Ticks ~hourly; for each ENABLED
/// cadence whose currently-due period has no report, it runs the skill (one run
/// at a time via an in-flight set).
pub struct InsightsScheduler {
    ctx: ServerCtx,
}

impl InsightsScheduler {
    pub fn new(ctx: ServerCtx) -> Self {
        Self { ctx }
    }

    /// Spawn the supervisor task. Returns a handle that cancels on drop.
    pub fn start(self) -> InsightsSchedulerHandle {
        let cancel = Arc::new(AtomicBool::new(false));
        let supervisor = tokio::spawn(self.supervise(Arc::clone(&cancel)));
        InsightsSchedulerHandle {
            cancel,
            _supervisor: supervisor,
        }
    }

    async fn supervise(self, cancel: Arc<AtomicBool>) {
        // In-flight set: cadences currently running (one run at a time overall,
        // but keyed by cadence so e.g. a slow weekly doesn't block a daily next
        // hour). `Mutex<HashSet>` mirrors otto-improve.
        let in_flight: Arc<Mutex<std::collections::HashSet<&'static str>>> =
            Arc::new(Mutex::new(std::collections::HashSet::new()));

        // Run the due-check immediately on startup (catch-up after the app was
        // closed), then once per hourly gate.
        let mut last_check: Option<std::time::Instant> = None;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }

            let gate_open = last_check
                .map(|t| t.elapsed() >= HOURLY_GATE)
                .unwrap_or(true);
            if gate_open {
                last_check = Some(std::time::Instant::now());
                self.tick(&in_flight).await;
            }

            // Sleep in short slices for responsive shutdown.
            let mut waited = Duration::ZERO;
            while waited < SCAN_INTERVAL {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                waited += Duration::from_millis(500);
            }
        }
    }

    /// One hourly due-check: for each enabled cadence, if the most-recent missed
    /// period isn't done and nothing's in flight for it, run it (offset 1).
    async fn tick(&self, in_flight: &Arc<Mutex<std::collections::HashSet<&'static str>>>) {
        let dir = insights_dir(&self.ctx);
        let cfg = read_config(&dir);
        let now = Utc::now();

        for kind in Kind::ALL {
            if !cfg.enabled(kind) {
                continue;
            }
            let (start, end) = due_period(kind, now);
            if period_done(&dir, kind, start, end) {
                continue;
            }
            // Skip if already running for this cadence.
            {
                let guard = in_flight.lock().await;
                if guard.contains(kind.word()) {
                    continue;
                }
            }
            in_flight.lock().await.insert(kind.word());

            let ctx = self.ctx.clone();
            let flight = Arc::clone(in_flight);
            let word = kind.word();
            info!(kind = word, "insights: scheduled catch-up run is due");
            tokio::spawn(async move {
                let session_id = match run_insights(&ctx, kind, 1).await {
                    Ok(Some(id)) => Some(id),
                    Ok(None) => {
                        // Skill not installed / no host — already logged inside.
                        None
                    }
                    Err(e) => {
                        warn!(kind = word, "insights: scheduled run failed: {e}");
                        None
                    }
                };
                // The session runs headlessly; release the in-flight slot after a
                // grace window so we don't re-trigger the same period mid-run
                // (idempotency would catch it once artifacts land, but this avoids
                // double-spawning before the index is written).
                tokio::time::sleep(RUN_TIMEOUT).await;
                flight.lock().await.remove(word);

                // After the grace window, check whether the period's report landed.
                // If so, emit `InsightReady` so the channel notifier + UI can react
                // without polling. Best-effort: a missed send is not an error.
                let (start, end) = due_period(kind, chrono::Utc::now());
                if period_done(&insights_dir(&ctx), kind, start, end) {
                    let period_label = format!("{} {}", word, start.format("%Y-%m-%d"));
                    let _ = ctx.events.send(Event::InsightReady {
                        period: period_label,
                        session_id,
                    });
                }
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    #[test]
    fn kind_period_and_word() {
        assert_eq!(Kind::Day.period(), "day");
        assert_eq!(Kind::Day.word(), "daily");
        assert_eq!(Kind::Week.period(), "week");
        assert_eq!(Kind::Week.word(), "weekly");
        assert_eq!(Kind::Month.period(), "month");
        assert_eq!(Kind::Month.word(), "monthly");
        assert_eq!(Kind::from_period("Week"), Some(Kind::Week));
        assert_eq!(Kind::from_period("monthly"), Some(Kind::Month));
        assert_eq!(Kind::from_period("year"), None);
    }

    #[test]
    fn due_day_is_yesterday() {
        let (s, e) = due_period(Kind::Day, at(2026, 6, 18));
        assert_eq!(s, NaiveDate::from_ymd_opt(2026, 6, 17).unwrap());
        assert_eq!(e, NaiveDate::from_ymd_opt(2026, 6, 17).unwrap());
    }

    #[test]
    fn due_week_is_previous_iso_week_mon_sun() {
        // 2026-06-18 is a Thursday. This ISO week = Mon 15 .. Sun 21.
        // Previous week = Mon 8 .. Sun 14.
        let (s, e) = due_period(Kind::Week, at(2026, 6, 18));
        assert_eq!(s, NaiveDate::from_ymd_opt(2026, 6, 8).unwrap());
        assert_eq!(e, NaiveDate::from_ymd_opt(2026, 6, 14).unwrap());
        assert_eq!(s.weekday(), chrono::Weekday::Mon);
        assert_eq!(e.weekday(), chrono::Weekday::Sun);
    }

    #[test]
    fn due_week_on_a_monday_uses_the_full_prior_week() {
        // 2026-06-15 is a Monday → previous week is Mon 8 .. Sun 14.
        let (s, e) = due_period(Kind::Week, at(2026, 6, 15));
        assert_eq!(s, NaiveDate::from_ymd_opt(2026, 6, 8).unwrap());
        assert_eq!(e, NaiveDate::from_ymd_opt(2026, 6, 14).unwrap());
    }

    #[test]
    fn due_month_is_previous_calendar_month() {
        let (s, e) = due_period(Kind::Month, at(2026, 6, 18));
        assert_eq!(s, NaiveDate::from_ymd_opt(2026, 5, 1).unwrap());
        assert_eq!(e, NaiveDate::from_ymd_opt(2026, 5, 31).unwrap());
    }

    #[test]
    fn due_month_january_rolls_to_previous_december() {
        let (s, e) = due_period(Kind::Month, at(2026, 1, 10));
        assert_eq!(s, NaiveDate::from_ymd_opt(2025, 12, 1).unwrap());
        assert_eq!(e, NaiveDate::from_ymd_opt(2025, 12, 31).unwrap());
    }

    #[test]
    fn period_key_format() {
        let s = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        let e = NaiveDate::from_ymd_opt(2026, 6, 16).unwrap();
        assert_eq!(period_key(Kind::Week, s, e), "weekly:20260610_20260616");
    }

    #[test]
    fn config_round_trips_and_defaults_off() {
        let dir = tempfile::tempdir().unwrap();
        // Absent file → all off.
        let cfg = read_config(dir.path());
        assert!(!cfg.daily && !cfg.weekly && !cfg.monthly);

        let on = InsightsConfig { daily: true, weekly: false, monthly: true };
        write_config(dir.path(), &on).unwrap();
        let read = read_config(dir.path());
        assert_eq!(read, on);
        assert!(read.enabled(Kind::Day));
        assert!(!read.enabled(Kind::Week));
        assert!(read.enabled(Kind::Month));
    }

    #[test]
    fn period_done_detects_html_metrics_and_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let s = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        let e = NaiveDate::from_ymd_opt(2026, 6, 16).unwrap();

        // Nothing on disk → not done.
        assert!(!period_done(root, Kind::Week, s, e));

        // A report HTML → done.
        let weekly = root.join("weekly");
        std::fs::create_dir_all(&weekly).unwrap();
        std::fs::write(weekly.join("report-weekly-20260610_20260616.html"), "<html>").unwrap();
        assert!(period_done(root, Kind::Week, s, e));

        // Different period still not done.
        let s2 = NaiveDate::from_ymd_opt(2026, 6, 3).unwrap();
        let e2 = NaiveDate::from_ymd_opt(2026, 6, 9).unwrap();
        assert!(!period_done(root, Kind::Week, s2, e2));

        // ... but an index.json row marks it done.
        std::fs::write(
            root.join("index.json"),
            serde_json::json!({
                "series": [{ "period_key": "weekly:20260603_20260609" }]
            })
            .to_string(),
        )
        .unwrap();
        assert!(period_done(root, Kind::Week, s2, e2));
    }

    #[test]
    fn list_reports_parses_and_sorts_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let weekly = root.join("weekly");
        std::fs::create_dir_all(&weekly).unwrap();

        // Older period.
        std::fs::write(weekly.join("report-weekly-20260601_20260607.html"), "<html>").unwrap();
        std::fs::write(
            weekly.join("summary-weekly-20260601_20260607.md"),
            "line1\nline2",
        )
        .unwrap();
        // Newer period (metrics only — still listed).
        std::fs::write(weekly.join("metrics-weekly-20260608_20260614.json"), "{}").unwrap();

        let reports = list_reports(root);
        assert_eq!(reports.len(), 2);
        // Newest first.
        assert_eq!(reports[0].period_start, "2026-06-08");
        assert_eq!(reports[0].period_end, "2026-06-14");
        assert!(reports[0].html_path.is_none());
        assert_eq!(reports[1].period_start, "2026-06-01");
        assert_eq!(reports[1].kind, "weekly");
        assert!(reports[1].html_path.is_some());
        assert_eq!(reports[1].summary, "line1\nline2");
    }

    #[test]
    fn parse_period_from_filename_accepts_all_three_prefixes() {
        assert_eq!(
            parse_period_from_filename("report-daily-20260617_20260617.html", "daily"),
            Some(("20260617".into(), "20260617".into()))
        );
        assert_eq!(
            parse_period_from_filename("summary-monthly-20260501_20260531.md", "monthly"),
            Some(("20260501".into(), "20260531".into()))
        );
        assert_eq!(
            parse_period_from_filename("metrics-weekly-20260608_20260614.json", "weekly"),
            Some(("20260608".into(), "20260614".into()))
        );
        // Wrong word for the dir → no match.
        assert_eq!(
            parse_period_from_filename("report-daily-20260617_20260617.html", "weekly"),
            None
        );
        // Junk file.
        assert_eq!(parse_period_from_filename("index.json", "weekly"), None);
    }
}
