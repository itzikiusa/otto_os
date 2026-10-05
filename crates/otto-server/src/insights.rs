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

use crate::cancel_signal::CancelSignal;
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightsConfig {
    #[serde(default)]
    pub daily: bool,
    #[serde(default)]
    pub weekly: bool,
    #[serde(default)]
    pub monthly: bool,
    /// Agent provider to generate reports on (built-in or custom, e.g. grok).
    /// Empty = the global default agent.
    #[serde(default)]
    pub provider: String,
    /// Optional model alias (empty = provider default).
    #[serde(default)]
    pub model: String,
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
            let first_this =
                NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
            let start = first_this
                .checked_sub_months(Months::new(1))
                .unwrap_or(first_this);
            // Last day of the previous month = day before the 1st of this month.
            let end = first_this.pred_opt().unwrap_or(start);
            (start, end)
        }
    }
}

/// Resolve the collector's host-local calendar window for a manual run. Unlike
/// the scheduler's UTC due check, this mirrors Python `datetime.now()` and
/// supports older offsets. The UI must not infer this in a remote browser's zone.
fn requested_period(kind: Kind, offset: i64, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let offset = u64::try_from(offset).ok()?;
    match kind {
        Kind::Day => {
            let day = today.checked_sub_days(Days::new(offset))?;
            Some((day, day))
        }
        Kind::Week => {
            let monday =
                today.checked_sub_days(Days::new(today.weekday().num_days_from_monday().into()))?;
            let start = monday.checked_sub_days(Days::new(offset.checked_mul(7)?))?;
            Some((start, start.checked_add_days(Days::new(6))?))
        }
        Kind::Month => {
            let first = today.with_day(1)?;
            let start = first.checked_sub_months(Months::new(u32::try_from(offset).ok()?))?;
            let end = start.checked_add_months(Months::new(1))?.pred_opt()?;
            Some((start, end))
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

/// A bounded list hydrates only the requested page, after sorting filenames.
/// Status polling never enumerates the archive.
#[derive(Debug, Default, Deserialize)]
pub struct ReportsQuery {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    #[serde(default)]
    pub summaries: bool,
    #[serde(default)]
    pub latest: bool,
}

pub fn list_reports(dir: &Path) -> Vec<ReportView> {
    report_page(
        dir,
        &ReportsQuery {
            summaries: true,
            ..Default::default()
        },
    )
}

#[allow(clippy::disallowed_methods)] // synchronous archive work runs on the blocking pool
fn report_page(dir: &Path, q: &ReportsQuery) -> Vec<ReportView> {
    let mut periods = std::collections::BTreeSet::new();
    for kind in Kind::ALL {
        let Ok(entries) = std::fs::read_dir(dir.join(kind.word())) else {
            continue;
        };
        for entry in entries.flatten() {
            if let Some((start, end)) =
                parse_period_from_filename(&entry.file_name().to_string_lossy(), kind.word())
            {
                periods.insert((end, start, kind.word()));
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    periods
        .into_iter()
        .rev()
        .filter(|(_, _, kind)| !q.latest || seen.insert(*kind))
        .skip(if q.latest { 0 } else { q.offset.unwrap_or(0) })
        .take(if q.latest {
            3
        } else {
            q.limit.unwrap_or(200).clamp(1, 200)
        })
        .filter_map(|(end, start, kind)| {
            report_status(dir, &format!("{kind}:{start}_{end}"), q.summaries)
                .ok()?
                .report
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct ReportStatus {
    pub report: Option<ReportView>,
    /// Only the HTML artifact determines completion; summary may land first.
    pub html_revision: Option<String>,
}

/// Three metadata reads and at most one bounded summary read, independent of
/// archive size. The strict key parser prevents directory traversal.
fn report_status(dir: &Path, key: &str, summary: bool) -> Result<ReportStatus, otto_core::Error> {
    let (kind, range) = key
        .split_once(':')
        .ok_or_else(|| otto_core::Error::Invalid("invalid report key".into()))?;
    let kind = Kind::from_period(kind)
        .ok_or_else(|| otto_core::Error::Invalid("invalid report kind".into()))?;
    let filename = format!("report-{}-{range}.html", kind.word());
    let (start, end) = parse_period_from_filename(&filename, kind.word())
        .ok_or_else(|| otto_core::Error::Invalid("invalid report period".into()))?;
    let stem = format!("{}-{start}_{end}", kind.word());
    let base = dir.join(kind.word());
    let html = base.join(format!("report-{stem}.html"));
    let summary_path = base.join(format!("summary-{stem}.md"));
    let metrics = base.join(format!("metrics-{stem}.json"));
    let meta: Vec<_> = [&html, &summary_path, &metrics]
        .into_iter()
        .map(|p| report_metadata(p).filter(|m| m.is_file()))
        .collect();
    let revision = |m: &std::fs::Metadata| {
        format!(
            "{}:{}",
            m.len(),
            m.modified().ok().map(fmt_systime).unwrap_or_default()
        )
    };
    let html_revision = meta[0].as_ref().map(revision);
    if meta.iter().all(Option::is_none) {
        return Ok(ReportStatus {
            report: None,
            html_revision,
        });
    }
    let created_at = meta
        .iter()
        .filter_map(|m| m.as_ref()?.modified().ok())
        .max()
        .map(fmt_systime)
        .unwrap_or_default();
    let text = if summary {
        summary_preview(&summary_path, meta[1].as_ref().map(revision))
    } else {
        String::new()
    };
    Ok(ReportStatus {
        report: Some(ReportView {
            kind: kind.word().into(),
            period_start: dashed(&start),
            period_end: dashed(&end),
            html_path: meta[0]
                .as_ref()
                .map(|_| html.to_string_lossy().into_owned()),
            summary: text,
            created_at,
        }),
        html_revision,
    })
}

#[cfg(test)]
thread_local! { static REPORT_IO: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) }; }

#[allow(clippy::disallowed_methods)] // called only by blocking report readers
fn report_metadata(path: &Path) -> Option<std::fs::Metadata> {
    #[cfg(test)]
    REPORT_IO.with(|c| {
        let (stats, reads) = c.get();
        c.set((stats + 1, reads));
    });
    std::fs::metadata(path).ok()
}

#[allow(clippy::disallowed_methods)] // bounded synchronous read on the blocking pool
fn summary_preview(path: &Path, revision: Option<String>) -> String {
    use std::io::Read;
    use std::sync::{Mutex as StdMutex, OnceLock};
    type Entry = (PathBuf, Option<String>, String);
    static CACHE: OnceLock<StdMutex<std::collections::VecDeque<Entry>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = cache
        .iter()
        .position(|(p, r, _)| p == path && r == &revision)
    {
        let entry = cache.remove(i).unwrap();
        let text = entry.2.clone();
        cache.push_back(entry);
        return text;
    }
    let mut bytes = Vec::new();
    #[cfg(test)]
    REPORT_IO.with(|c| {
        let (stats, reads) = c.get();
        c.set((stats, reads + 1));
    });
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(64 * 1024).read_to_end(&mut bytes);
    }
    let text = truncate_lines(&String::from_utf8_lossy(&bytes), 80);
    cache.retain(|(p, _, _)| p != path);
    cache.push_back((path.to_owned(), revision, text.clone()));
    while cache.len() > 64 {
        cache.pop_front();
    }
    text
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
    // Both halves must be pure YYYYMMDD digits — these strings are re-joined
    // into on-disk paths by `list_reports`, so anything else is rejected here.
    if start.len() == 8
        && end.len() == 8
        && start.chars().all(|c| c.is_ascii_digit())
        && end.chars().all(|c| c.is_ascii_digit())
    {
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

/// Manual requests explicitly regenerate; catch-up keeps per-period idempotency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Manual,
    Scheduled,
}

/// Build the headless prompt that drives the `insights` skill for one period.
pub fn build_run_prompt(
    kind: Kind,
    offset: i64,
    as_of: NaiveDate,
    collector: &Path,
    mode: RunMode,
) -> String {
    format!(
        "Run the `insights` skill to generate the usage report for the {period} \
         period at --offset {offset} (the previous {period} when offset is 1). \
         Use the pinned collector at {collector} for every collection step (including \
         facet extraction and re-collection) with `--period {period} --offset {offset} \
         --as-of {as_of}{force}`. Keep this calendar reference even if the date changes. Follow \
         the skill's full method (collect, classify facet-less sessions, compare \
         to the prior comparable period, render the self-contained HTML report), \
         and store all three artifacts (report HTML, summary markdown, metrics \
         JSON) plus update index.json. The report is for the period that ended; \
         do not ask the user any questions — run it end-to-end and stop when the \
         report is written. {existing}",
        period = kind.period(),
        offset = offset,
        as_of = as_of,
        force = if mode == RunMode::Manual {
            " --force"
        } else {
            ""
        },
        existing = if mode == RunMode::Manual {
            "The user explicitly requested generation: replace this period's report even if it already exists. Retain --force for every collection step."
        } else {
            "If the period was already generated, note that and stop."
        },
        collector = serde_json::to_string(&collector.to_string_lossy()).unwrap_or_default(),
    )
}

/// A daemon-owned, content-addressed collector keeps the calendar contract even
/// when the installed skill is older. Never edits the user's skill directory.
fn materialize_collector(dir: &Path) -> std::io::Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    let source = otto_skills::bundled_file(INSIGHTS_SKILL, "scripts/collect_insights.py")
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "bundled insights collector missing",
            )
        })?;
    let collectors = dir.join("collectors");
    std::fs::create_dir_all(&collectors)?;
    let path = collectors.join(format!(
        "{}.py",
        hex::encode(Sha256::digest(source.as_bytes()))
    ));
    let mut pending = tempfile::NamedTempFile::new_in(&collectors)?;
    pending.write_all(source.as_bytes())?;
    match pending.persist_noclobber(&path) {
        Ok(_) => {}
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read_to_string(&path)? != source {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "pinned insights collector changed",
                ));
            }
        }
        Err(e) => return Err(e.error),
    }
    Ok(path)
}

/// Spawn a real, openable agent session that runs the `insights` skill for
/// `(kind, offset)`. The session uses the global default provider, runs in a
/// neutral cwd (the Otto data dir), and is NOT awaited to completion here — it
/// runs headlessly like the planner / product lens sessions.
///
/// Returns the spawned session id on success. Returns `Ok(None)` (a no-op) when
/// the `insights` skill is not installed in the Library — the caller logs a
/// warning and the UI tells the user to install it.
pub async fn run_insights(
    ctx: &ServerCtx,
    kind: Kind,
    offset: i64,
    as_of: NaiveDate,
    mode: RunMode,
) -> otto_core::Result<Option<otto_core::Id>> {
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

    // Provider/model come from the insights config (built-in or custom, e.g.
    // grok); an empty provider falls back to the host workspace's default agent,
    // then the global default.
    let cfg = read_config(&insights_dir(ctx));
    let provider = if cfg.provider.trim().is_empty() {
        ctx.resolve_provider(Some(&ws), None).await?
    } else {
        cfg.provider.trim().to_string()
    };
    otto_sessions::trust::ensure_trusted(&provider, &cwd);

    let collector = materialize_collector(&insights_dir(ctx))
        .map_err(|e| otto_core::Error::Internal(format!("prepare insights collector: {e}")))?;
    let prompt = build_run_prompt(kind, offset, as_of, &collector, mode);
    let mut meta = serde_json::json!({ "source": "insights" });
    if !cfg.model.trim().is_empty() {
        meta["model"] = serde_json::json!(cfg.model.trim());
    }
    let req = otto_core::api::CreateSessionReq {
        kind: otto_core::domain::SessionKind::Agent,
        provider: Some(provider.clone()),
        title: Some(format!("Insights: {}", kind.word())),
        cwd: Some(cwd.clone()),
        connection_id: None,
        model: None,
        meta: Some(meta),
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
/// Prefers a root member; falls back to any member. User-facing list: the
/// scratch workspace is created at boot (so it would sort first) and has no
/// members, which would leave insights with no host on a fresh install.
async fn pick_host(ctx: &ServerCtx) -> Option<(otto_core::domain::Workspace, otto_core::Id)> {
    let workspaces = ctx.workspaces.list_user_all().await.ok()?;
    let ws = workspaces.into_iter().find(|w| !w.archived)?;
    let members = ctx.workspaces.members(&ws.id).await.ok()?;
    // Prefer the workspace admin/first member as the actor.
    let user_id = members.first().map(|m| m.user_id.clone())?;
    Some((ws, user_id))
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
    /// Requested collector period, resolved in the daemon's local timezone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_key: Option<String>,
    pub report_revision: Option<String>,
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
        .route("/insights/report-status", get(get_report_status))
        .route("/insights/run", post(post_run))
}

async fn get_config(State(ctx): State<ServerCtx>) -> ApiResult<Json<InsightsConfig>> {
    let dir = insights_dir(&ctx);
    // std::fs off the async workers, like every other insights file read.
    let cfg = tokio::task::spawn_blocking(move || read_config(&dir))
        .await
        .map_err(|e| {
            ApiError(otto_core::Error::Internal(format!(
                "read insights config: {e}"
            )))
        })?;
    Ok(Json(cfg))
}

async fn put_config(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(cfg): Json<InsightsConfig>,
) -> ApiResult<Json<InsightsConfig>> {
    require_root(&user)?;
    let dir = insights_dir(&ctx);
    let to_write = cfg.clone();
    tokio::task::spawn_blocking(move || write_config(&dir, &to_write))
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))
        .and_then(|r| r)
        .map_err(|e| {
            ApiError(otto_core::Error::Internal(format!(
                "write insights config: {e}"
            )))
        })?;
    Ok(Json(cfg))
}

async fn get_reports(
    State(ctx): State<ServerCtx>,
    Query(q): Query<ReportsQuery>,
) -> ApiResult<Json<Vec<ReportView>>> {
    let dir = insights_dir(&ctx);
    // ~300 reads + ~900 stats of synchronous std::fs: off the async runtime
    // (the Insights page polls this every 3 s during a run — backlog B6 / SE-17).
    let reports = tokio::task::spawn_blocking(move || report_page(&dir, &q))
        .await
        .map_err(|e| {
            ApiError(otto_core::Error::Internal(format!(
                "list insights reports: {e}"
            )))
        })?;
    Ok(Json(reports))
}

#[derive(Deserialize)]
struct StatusQuery {
    key: String,
    #[serde(default)]
    summary: bool,
}

async fn get_report_status(
    State(ctx): State<ServerCtx>,
    Query(q): Query<StatusQuery>,
) -> ApiResult<Json<ReportStatus>> {
    let dir = insights_dir(&ctx);
    let status = crate::offload::blocking(move || report_status(&dir, &q.key, q.summary))
        .await
        .map_err(ApiError)?;
    Ok(Json(status))
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
    // Canonicalize + read on the blocking pool, not a runtime worker.
    let dir = insights_dir(&ctx);
    let html = tokio::task::spawn_blocking(move || -> Result<String, ApiError> {
        let base = std::fs::canonicalize(dir)
            .map_err(|e| ApiError(otto_core::Error::Internal(format!("insights dir: {e}"))))?;
        let req = std::fs::canonicalize(Path::new(&q.path))
            .map_err(|_| ApiError(otto_core::Error::NotFound("report".into())))?;
        if !req.starts_with(&base) {
            return Err(ApiError(otto_core::Error::Forbidden(
                "path is outside the insights directory".into(),
            )));
        }
        std::fs::read_to_string(&req)
            .map_err(|_| ApiError(otto_core::Error::NotFound("report".into())))
    })
    .await
    .map_err(|e| ApiError(otto_core::Error::Internal(format!("join: {e}"))))??;
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
    let as_of = chrono::Local::now().date_naive();
    let (start, end) = requested_period(kind, offset, as_of).ok_or_else(|| {
        ApiError(otto_core::Error::Invalid(
            "report offset is out of range".into(),
        ))
    })?;

    let dir = insights_dir(&ctx);
    let key = period_key(kind, start, end);
    let report_revision = crate::offload::blocking(move || report_status(&dir, &key, false))
        .await
        .map_err(ApiError)?
        .html_revision;

    match run_insights(&ctx, kind, offset, as_of, RunMode::Manual).await {
        Ok(Some(id)) => Ok(Json(RunResp {
            started: true,
            run_id: Some(id.to_string()),
            report_key: Some(period_key(kind, start, end)),
            report_revision,
            reason: None,
        })),
        Ok(None) => Ok(Json(RunResp {
            started: false,
            run_id: None,
            report_key: None,
            report_revision: None,
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
    cancel: CancelSignal,
    _supervisor: JoinHandle<()>,
}

impl InsightsSchedulerHandle {
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

impl Drop for InsightsSchedulerHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
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
        let cancel = CancelSignal::new();
        let supervisor = tokio::spawn(self.supervise(cancel.clone()));
        InsightsSchedulerHandle {
            cancel,
            _supervisor: supervisor,
        }
    }

    async fn supervise(self, cancel: CancelSignal) {
        // In-flight set: cadences currently running (one run at a time overall,
        // but keyed by cadence so e.g. a slow weekly doesn't block a daily next
        // hour). `Mutex<HashSet>` mirrors otto-improve.
        let in_flight: Arc<Mutex<std::collections::HashSet<&'static str>>> =
            Arc::new(Mutex::new(std::collections::HashSet::new()));

        // Run the due-check immediately on startup (catch-up after the app was
        // closed), then once per hourly gate.
        let mut last_check: Option<std::time::Instant> = None;
        loop {
            if cancel.is_cancelled() {
                return;
            }

            let gate_open = last_check
                .map(|t| t.elapsed() >= HOURLY_GATE)
                .unwrap_or(true);
            if gate_open {
                last_check = Some(std::time::Instant::now());
                self.tick(&in_flight).await;
            }

            // One timer per scan; cancel() wakes it (was 500 ms slices).
            if cancel.sleep(SCAN_INTERVAL).await {
                return;
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
                let session_id =
                    match run_insights(&ctx, kind, 1, now.date_naive(), RunMode::Scheduled).await {
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

    #[test]
    fn status_io_is_constant_for_300_and_1000_reports_and_requires_new_html() {
        let tmp = tempfile::tempdir().unwrap();
        let daily = tmp.path().join("daily");
        std::fs::create_dir(&daily).unwrap();
        let first = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        for i in 0..1000 {
            let day = first
                .checked_add_days(Days::new(i))
                .unwrap()
                .format("%Y%m%d")
                .to_string();
            for (prefix, ext) in [("report", "html"), ("summary", "md"), ("metrics", "json")] {
                std::fs::write(
                    daily.join(format!("{prefix}-daily-{day}_{day}.{ext}")),
                    "one",
                )
                .unwrap();
            }
            if i == 299 || i == 999 {
                REPORT_IO.with(|c| c.set((0, 0)));
                for _ in 0..10 {
                    report_status(tmp.path(), "daily:20200101_20200101", false).unwrap();
                }
                assert_eq!(REPORT_IO.with(|c| c.get()), (30, 0));
            }
        }
        let key = "daily:20200101_20200101";
        let before = report_status(tmp.path(), key, false).unwrap().html_revision;
        std::fs::write(
            daily.join("summary-daily-20200101_20200101.md"),
            "new summary first",
        )
        .unwrap();
        assert_eq!(
            report_status(tmp.path(), key, true).unwrap().html_revision,
            before
        );
        std::fs::write(
            daily.join("report-daily-20200101_20200101.html"),
            "new html completed",
        )
        .unwrap();
        assert_ne!(
            report_status(tmp.path(), key, false).unwrap().html_revision,
            before
        );
        REPORT_IO.with(|c| c.set((0, 0)));
        let page = report_page(
            tmp.path(),
            &ReportsQuery {
                limit: Some(20),
                ..Default::default()
            },
        );
        assert_eq!(page.len(), 20);
        assert!(page.iter().all(|r| r.summary.is_empty()));
        assert_eq!(REPORT_IO.with(|c| c.get()), (60, 0));
        REPORT_IO.with(|c| c.set((0, 0)));
        assert_eq!(
            report_page(
                tmp.path(),
                &ReportsQuery {
                    latest: true,
                    summaries: true,
                    ..Default::default()
                }
            )
            .len(),
            1
        );
        assert_eq!(REPORT_IO.with(|c| c.get()).0, 3);
    }

    #[test]
    fn previews_are_bounded_cached_and_refresh_after_external_edit() {
        let tmp = tempfile::tempdir().unwrap();
        let daily = tmp.path().join("daily");
        std::fs::create_dir(&daily).unwrap();
        let path = daily.join("summary-daily-20200101_20200101.md");
        std::fs::write(&path, "x".repeat(1024 * 1024)).unwrap();
        let key = "daily:20200101_20200101";
        REPORT_IO.with(|c| c.set((0, 0)));
        assert!(
            report_status(tmp.path(), key, true)
                .unwrap()
                .report
                .unwrap()
                .summary
                .len()
                <= 65536
        );
        report_status(tmp.path(), key, true).unwrap();
        assert_eq!(REPORT_IO.with(|c| c.get()), (6, 1));
        std::fs::write(path, "external change").unwrap();
        assert_eq!(
            report_status(tmp.path(), key, true)
                .unwrap()
                .report
                .unwrap()
                .summary,
            "external change"
        );
        assert!(report_status(tmp.path(), "daily:../../outside", false).is_err());
    }
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    #[test]
    fn pinned_collector_preserves_legacy_skill_and_concurrent_materialization() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("library/skills/insights");
        std::fs::create_dir_all(installed.join("scripts")).unwrap();
        std::fs::write(installed.join("SKILL.md"), "version: 2\nCustom narrative").unwrap();
        std::fs::write(
            installed.join("scripts/collect_insights.py"),
            "# legacy custom collector",
        )
        .unwrap();
        let dir = root.path().join("insights");
        let paths = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| scope.spawn(|| materialize_collector(&dir).unwrap()))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(paths.iter().all(|p| p == &paths[0]));
        assert!(std::fs::read_to_string(&paths[0])
            .unwrap()
            .contains("--as-of"));
        assert_eq!(
            std::fs::read_to_string(installed.join("scripts/collect_insights.py")).unwrap(),
            "# legacy custom collector"
        );
        assert_eq!(
            std::fs::read_to_string(installed.join("SKILL.md")).unwrap(),
            "version: 2\nCustom narrative"
        );
        let prompt = build_run_prompt(
            Kind::Month,
            1,
            NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            &paths[0],
            RunMode::Scheduled,
        );
        assert!(prompt.contains("--period month --offset 1 --as-of 2026-09-30"));
        assert!(prompt.contains(paths[0].to_str().unwrap()));
        std::fs::write(&paths[0], "modified").unwrap();
        assert!(materialize_collector(&dir).is_err());
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "modified");
    }

    #[test]
    fn manual_regeneration_overrides_existing_report_while_scheduler_keeps_idempotency() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let path = Path::new("/tmp/synthetic/collector.py");
        let manual = build_run_prompt(Kind::Day, 1, date, path, RunMode::Manual);
        let scheduled = build_run_prompt(Kind::Day, 1, date, path, RunMode::Scheduled);
        assert!(manual.contains("--as-of 2026-09-30 --force"));
        assert!(manual.contains("replace this period's report even if it already exists"));
        assert!(!manual.contains("note that and stop"));
        assert!(!scheduled.contains("--force"));
        assert!(scheduled.contains("If the period was already generated, note that and stop"));
    }

    #[test]
    fn requested_period_matches_collector_calendar_and_offset() {
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let key = |kind, offset| {
            let (start, end) = requested_period(kind, offset, today).unwrap();
            period_key(kind, start, end)
        };
        assert_eq!(key(Kind::Day, 2), "daily:20251230_20251230");
        assert_eq!(key(Kind::Week, 1), "weekly:20251222_20251228");
        assert_eq!(key(Kind::Month, 2), "monthly:20251101_20251130");
        assert_eq!(key(Kind::Day, 0), "daily:20260101_20260101");
        let leap = NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();
        let (start, end) = requested_period(Kind::Month, 1, leap).unwrap();
        assert_eq!(
            period_key(Kind::Month, start, end),
            "monthly:20240201_20240229"
        );
        assert!(requested_period(Kind::Week, i64::MAX, today).is_none());
        assert!(requested_period(Kind::Day, -1, today).is_none());
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

        let on = InsightsConfig {
            daily: true,
            weekly: false,
            monthly: true,
            ..Default::default()
        };
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
        std::fs::write(
            weekly.join("report-weekly-20260610_20260616.html"),
            "<html>",
        )
        .unwrap();
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
        std::fs::write(
            weekly.join("report-weekly-20260601_20260607.html"),
            "<html>",
        )
        .unwrap();
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
