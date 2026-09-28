//! Daily auto-update of the agent CLIs (Claude Code, Codex, …), with optional
//! force-reload of open agent sessions so they re-exec on the new binary.
//!
//! Mirrors the insights scheduler: a lightweight supervisor ticks every minute,
//! and a `last_run` cursor makes it **catch up a missed window** — if the machine
//! was asleep/off through the scheduled time, the job runs at the next
//! opportunity instead of being skipped. Everything is user-configurable via the
//! `cli_auto_update` setting (enable, time-of-day, reload-sessions) and can be
//! turned off.
//!
//! Why reload sessions at all? A running CLI is the *old* binary already loaded
//! into memory — updating the file on disk does nothing to the live process. The
//! only way to pick up a new version is to re-exec it. `SessionManager::restart`
//! does exactly that and is resume-aware (replays `--resume <provider_session_id>`),
//! so the conversation continues uninterrupted.
//!
//! Two guards keep that reload from being a nightly disruption (r3-08-01):
//! - **Only on a real change.** `claude update` & co. exit 0 when already
//!   current, so "exit 0" is not "updated". Each provider's `<program>
//!   --version` is probed before and after its updater; only providers whose
//!   version string CHANGED (or could not be probed) get their sessions
//!   reloaded.
//! - **Never mid-turn.** A session that is working, streaming output, held
//!   by an engine turn or has a fresh open provider turn
//!   ([`SessionManager::busy_for_restart`]) is deferred: a background waiter
//!   re-checks it every [`DEFER_POLL`] and restarts it once idle — unless its
//!   process was already replaced (a user restart picked the new binary up)
//!   or it went away. Restarts are staggered by [`RESTART_STAGGER`].

use std::collections::HashSet;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, TimeZone, Utc};
use otto_core::domain::{NoticeKind, NoticeSeverity, SessionKind};
use otto_state::{NewNotice, SettingsRepo};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::cancel_signal::CancelSignal;
use crate::state::ServerCtx;

/// Settings key holding the user config object.
const CONFIG_KEY: &str = "cli_auto_update";
/// Settings key holding the internal `last_run` cursor (kept separate so a UI
/// config save never clobbers it).
const LAST_RUN_KEY: &str = "cli_auto_update_last_run";
/// De-dupe key for the summary notice.
const NOTICE_KEY: &str = "cli_auto_update";

/// Supervisor tick (the due-check is cheap + idempotent via `last_run`).
const TICK: Duration = Duration::from_secs(60);
/// Hard cap on a single update run so a hung CLI can't wedge the scheduler.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Cap on one `<program> --version` probe.
const VERSION_TIMEOUT: Duration = Duration::from_secs(20);
/// How often a deferred (busy) session is re-checked for idleness.
const DEFER_POLL: Duration = Duration::from_secs(30);
/// Give up on a deferred reload after this long (the next manual restart or
/// the next nightly run picks the new binary up instead).
const DEFER_MAX: Duration = Duration::from_secs(12 * 60 * 60);
/// Gap between two session restarts (each is a CLI cold start + MCP children).
const RESTART_STAGGER: Duration = Duration::from_secs(3);

fn default_enabled() -> bool {
    true
}
fn default_time() -> String {
    // 03:00 UTC — new CLI versions are typically published by then.
    "03:00".to_string()
}
fn default_use_utc() -> bool {
    true
}
fn default_reload() -> bool {
    true
}

/// User-facing configuration (persisted under [`CONFIG_KEY`], edited in Settings).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliAutoUpdateConfig {
    /// Master switch — off means the scheduler never runs.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Time-of-day to run, `"HH:MM"` (24h), interpreted in UTC or local per
    /// [`Self::use_utc`].
    #[serde(default = "default_time")]
    pub time_of_day: String,
    /// Interpret `time_of_day` as UTC (default — matches when vendors publish)
    /// rather than the machine's local timezone.
    #[serde(default = "default_use_utc")]
    pub use_utc: bool,
    /// After updating, restart open agent sessions so they load the new binary.
    #[serde(default = "default_reload")]
    pub reload_sessions: bool,
}

impl Default for CliAutoUpdateConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            time_of_day: default_time(),
            use_utc: default_use_utc(),
            reload_sessions: default_reload(),
        }
    }
}

impl CliAutoUpdateConfig {
    async fn load(settings: &SettingsRepo) -> Self {
        match settings.get(CONFIG_KEY).await {
            Ok(Some(v)) => serde_json::from_value(v).unwrap_or_default(),
            _ => Self::default(),
        }
    }
}

/// Parse `"HH:MM"` into `(hour, minute)`, both in range.
fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    (h < 24 && m < 60).then_some((h, m))
}

/// The scheduled instant for "today" (UTC or local per `use_utc`), as UTC.
fn scheduled_today(
    now: DateTime<Utc>,
    use_utc: bool,
    hour: u32,
    minute: u32,
) -> Option<DateTime<Utc>> {
    if use_utc {
        let naive = now.date_naive().and_hms_opt(hour, minute, 0)?;
        Some(Utc.from_utc_datetime(&naive))
    } else {
        let now_local = now.with_timezone(&Local);
        let naive = now_local.date_naive().and_hms_opt(hour, minute, 0)?;
        // DST gap/fold → no single local instant; skip this tick, try the next.
        Local
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.with_timezone(&Utc))
    }
}

/// Is the job due to run now? True when we're at/after today's scheduled time
/// AND we haven't already run for today's window. The `last_run` check is what
/// delivers catch-up: after a missed window the daemon comes up "past" the time
/// with `last_run` still from a previous day, so it fires immediately. All
/// comparisons are in UTC.
fn is_due(
    now: DateTime<Utc>,
    use_utc: bool,
    hour: u32,
    minute: u32,
    last_run: Option<DateTime<Utc>>,
) -> bool {
    let scheduled = match scheduled_today(now, use_utc, hour, minute) {
        Some(dt) => dt,
        None => return false,
    };
    if now < scheduled {
        return false;
    }
    match last_run {
        None => true,
        Some(lr) => lr < scheduled,
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

pub struct CliUpdateSchedulerHandle {
    cancel: CancelSignal,
    _supervisor: JoinHandle<()>,
}

impl CliUpdateSchedulerHandle {
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

impl Drop for CliUpdateSchedulerHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub struct CliUpdateScheduler {
    ctx: ServerCtx,
}

impl CliUpdateScheduler {
    pub fn new(ctx: ServerCtx) -> Self {
        Self { ctx }
    }

    /// Spawn the supervisor; returns a handle that cancels on drop.
    pub fn start(self) -> CliUpdateSchedulerHandle {
        let cancel = CancelSignal::new();
        let supervisor = tokio::spawn(self.supervise(cancel.clone()));
        CliUpdateSchedulerHandle {
            cancel,
            _supervisor: supervisor,
        }
    }

    async fn supervise(self, cancel: CancelSignal) {
        // Single in-flight guard: a run takes minutes; never overlap.
        let running = Arc::new(AtomicBool::new(false));
        loop {
            if cancel.is_cancelled() {
                return;
            }
            if !running.load(Ordering::Relaxed) {
                self.tick(&running).await;
            }
            // One timer per tick; shutdown()/drop wakes it (SG-12: no 500 ms
            // polling slices).
            if cancel.sleep(TICK).await {
                return;
            }
        }
    }

    async fn tick(&self, running: &Arc<AtomicBool>) {
        let settings = SettingsRepo::new(self.ctx.pool.clone());
        let cfg = CliAutoUpdateConfig::load(&settings).await;
        if !cfg.enabled {
            return;
        }
        let Some((hour, minute)) = parse_hhmm(&cfg.time_of_day) else {
            warn!(time = %cfg.time_of_day, "cli_auto_update: invalid time_of_day, skipping");
            return;
        };
        let last_run = read_last_run(&settings).await;
        if !is_due(Utc::now(), cfg.use_utc, hour, minute, last_run) {
            return;
        }

        // Claim the slot, then run detached so the supervisor stays responsive.
        running.store(true, Ordering::Relaxed);
        let ctx = self.ctx.clone();
        let running = Arc::clone(running);
        info!("cli_auto_update: scheduled run is due");
        tokio::spawn(async move {
            run(&ctx, &cfg).await;
            running.store(false, Ordering::Relaxed);
        });
    }
}

async fn read_last_run(settings: &SettingsRepo) -> Option<DateTime<Utc>> {
    let v = settings.get(LAST_RUN_KEY).await.ok()??;
    let s = v.as_str()?;
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

// ---------------------------------------------------------------------------
// The run: update CLIs → reload sessions → record → notify
// ---------------------------------------------------------------------------

/// One provider's update result: `detail` is empty on success, else the short
/// human-readable reason (exit + stderr tail / launch error / timeout).
#[derive(Debug, Clone)]
struct UpdateOutcome {
    name: String,
    ok: bool,
    detail: String,
    /// `<program> --version` before / after the updater (None = probe failed).
    before: Option<String>,
    after: Option<String>,
}

impl UpdateOutcome {
    /// Did this run actually put a different binary in place? Unknown (a
    /// probe failed) counts as changed — the reload is deferred to idle
    /// anyway, so the conservative answer costs one quiet restart at most.
    fn changed(&self) -> bool {
        self.ok && version_changed(self.before.as_deref(), self.after.as_deref())
    }
}

/// Pure version comparison (see [`UpdateOutcome::changed`]).
fn version_changed(before: Option<&str>, after: Option<&str>) -> bool {
    match (before, after) {
        (Some(b), Some(a)) => b != a,
        _ => true,
    }
}

/// Reload tally for the notice: restarted now, failed, deferred until idle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ReloadTally {
    reloaded: u32,
    failed: u32,
    deferred: u32,
}

/// Run one update pass. Public so a future "run now" route can reuse it.
pub async fn run(ctx: &ServerCtx, cfg: &CliAutoUpdateConfig) {
    let settings = SettingsRepo::new(ctx.pool.clone());

    let mut pairs = ctx.manager.provider_update_commands();
    if pairs.is_empty() {
        info!("cli_auto_update: no providers have an update command; nothing to do");
        let _ = settings.put(LAST_RUN_KEY, &json_now()).await;
        return;
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    // Each provider updates in its OWN shell invocation so one CLI's failure
    // can't mask another's status or get mis-attributed in the notice (a single
    // compound command only surfaces the LAST command's exit code).
    let mut outcomes: Vec<UpdateOutcome> = Vec::with_capacity(pairs.len());
    for (name, cmd) in &pairs {
        let program = ctx.manager.provider_program(name);
        let before = match &program {
            Some(p) => probe_version(p).await,
            None => None,
        };
        let (ok, detail) = run_one_update(cmd).await;
        let after = match (&program, ok) {
            (Some(p), true) => probe_version(p).await,
            _ => before.clone(),
        };
        info!(
            provider = %name,
            ok,
            before = before.as_deref().unwrap_or("?"),
            after = after.as_deref().unwrap_or("?"),
            "cli_auto_update: update finished"
        );
        outcomes.push(UpdateOutcome {
            name: name.clone(),
            ok,
            detail,
            before,
            after,
        });
    }

    // Reload open agent sessions — only for providers whose update SUCCEEDED
    // and actually CHANGED the binary (re-exec onto the same version, or onto
    // one whose update just failed, helps nobody and kills live turns).
    let updated_names: HashSet<String> = outcomes
        .iter()
        .filter(|o| o.changed())
        .map(|o| o.name.clone())
        .collect();
    let reload = if cfg.reload_sessions && !updated_names.is_empty() {
        Some(reload_agent_sessions(ctx, &updated_names).await)
    } else {
        None
    };

    let _ = settings.put(LAST_RUN_KEY, &json_now()).await;

    // Summary notice (system-wide, de-duped by source key).
    let any_update_failed = outcomes.iter().any(|o| !o.ok);
    let severity = if !any_update_failed && reload.is_none_or(|t| t.failed == 0) {
        NoticeSeverity::Info
    } else {
        NoticeSeverity::Warn
    };
    let _ = ctx
        .notifications()
        .create(NewNotice {
            kind: NoticeKind::System,
            severity,
            title: "Daily CLI update".to_string(),
            body: build_body(&outcomes, reload),
            source_key: Some(NOTICE_KEY.to_string()),
            action: None,
            user_id: None,
        })
        .await;
}

/// Compose the notice body from per-provider outcomes: only providers that
/// actually updated are listed as updated, and each failure names its provider
/// and reason. Pure, so the exact wording is unit-tested.
fn build_body(outcomes: &[UpdateOutcome], reload: Option<ReloadTally>) -> String {
    let updated: Vec<String> = outcomes
        .iter()
        .filter(|o| o.changed())
        .map(|o| match (&o.before, &o.after) {
            (Some(b), Some(a)) => format!("{} ({b} → {a})", o.name),
            _ => o.name.clone(),
        })
        .collect();
    let current: Vec<&str> = outcomes
        .iter()
        .filter(|o| o.ok && !o.changed())
        .map(|o| o.name.as_str())
        .collect();
    let failed: Vec<String> = outcomes
        .iter()
        .filter(|o| !o.ok)
        .map(|o| format!("{} — {}", o.name, o.detail))
        .collect();

    let mut body = if updated.is_empty() {
        "No CLIs updated.".to_string()
    } else {
        format!("Updated CLIs: {}.", updated.join(", "))
    };
    if !current.is_empty() {
        body.push_str(&format!(" Already up to date: {}.", current.join(", ")));
    }
    if !failed.is_empty() {
        body.push_str(&format!(" Failed: {}.", failed.join("; ")));
    }
    if let Some(t) = reload {
        body.push_str(&format!(" Reloaded {} open session(s)", t.reloaded));
        if t.failed > 0 {
            body.push_str(&format!(" ({} failed)", t.failed));
        }
        if t.deferred > 0 {
            body.push_str(&format!(
                "; {} busy session(s) reload when idle",
                t.deferred
            ));
        }
        body.push('.');
    }
    body
}

/// `'<program>' --version` through the same login shell the updater uses (so
/// PATH resolves the same binary). First non-empty stdout line, ≤200 chars;
/// `None` on failure/timeout — callers treat that as "unknown".
async fn probe_version(program: &str) -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let quoted = format!("'{}' --version", program.replace('\'', "'\\''"));
    let fut = tokio::process::Command::new(&shell)
        .arg("-l")
        .arg("-c")
        .arg(&quoted)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(VERSION_TIMEOUT, fut)
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(200).collect())
}

/// Run ONE provider's update command via a login shell (so the user's
/// PATH/profile resolves the CLI), bounded by a timeout. Returns
/// `(success, short_detail)` — detail is empty on success.
async fn run_one_update(cmd: &str) -> (bool, String) {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let fut = tokio::process::Command::new(&shell)
        .arg("-l")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match tokio::time::timeout(UPDATE_TIMEOUT, fut).await {
        Ok(Ok(out)) if out.status.success() => (true, String::new()),
        Ok(Ok(out)) => {
            let tail = String::from_utf8_lossy(&out.stderr);
            let tail = tail.trim();
            let snippet: String = tail.chars().rev().take(200).collect::<String>();
            let snippet: String = snippet.chars().rev().collect();
            (
                false,
                format!("exit {}: {snippet}", out.status.code().unwrap_or(-1)),
            )
        }
        Ok(Err(e)) => (false, format!("failed to launch updater: {e}")),
        Err(_) => (false, "update timed out".to_string()),
    }
}

/// Restart every live AGENT session whose provider was updated. Connection PTYs
/// (ssh/db) and plain shells are left alone. `restart` auto-resumes. Busy
/// sessions are NOT interrupted: they are handed to [`reload_when_idle`].
async fn reload_agent_sessions(ctx: &ServerCtx, providers: &HashSet<String>) -> ReloadTally {
    let mut tally = ReloadTally::default();
    let mut deferred: Vec<(otto_core::Id, Option<u32>)> = Vec::new();
    let workspaces = match ctx.workspaces.list_all().await {
        Ok(w) => w,
        Err(e) => {
            warn!("cli_auto_update: list workspaces failed: {e}");
            return tally;
        }
    };
    for ws in workspaces {
        let sessions = match ctx.manager.list_by_workspace(&ws.id).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        for s in sessions {
            if s.kind != SessionKind::Agent
                || !providers.contains(&s.provider)
                || !ctx.manager.is_live(&s.id)
            {
                continue;
            }
            if ctx.manager.busy_for_restart(&s.id).await {
                info!(session = %s.id, provider = %s.provider, "cli_auto_update: session busy, reload deferred until idle");
                deferred.push((s.id.clone(), ctx.manager.live_pid(&s.id)));
                continue;
            }
            if tally.reloaded + tally.failed > 0 {
                tokio::time::sleep(RESTART_STAGGER).await;
            }
            match ctx.manager.restart(&s.id, None).await {
                Ok(_) => {
                    tally.reloaded += 1;
                    info!(session = %s.id, provider = %s.provider, "cli_auto_update: reloaded session");
                }
                Err(e) => {
                    tally.failed += 1;
                    warn!(session = %s.id, "cli_auto_update: reload failed: {e}");
                }
            }
        }
    }
    tally.deferred = deferred.len() as u32;
    if !deferred.is_empty() {
        let manager = ctx.manager.clone();
        tokio::spawn(reload_when_idle(manager, deferred));
    }
    tally
}

/// Background waiter for sessions that were mid-turn at reload time: every
/// [`DEFER_POLL`] restart the ones that went idle; drop the ones that died or
/// whose process was already replaced (pid changed ⇒ the new binary is
/// running); give up after [`DEFER_MAX`].
async fn reload_when_idle(
    manager: Arc<otto_sessions::SessionManager>,
    mut pending: Vec<(otto_core::Id, Option<u32>)>,
) {
    let started = std::time::Instant::now();
    while !pending.is_empty() && started.elapsed() < DEFER_MAX {
        tokio::time::sleep(DEFER_POLL).await;
        let mut still = Vec::with_capacity(pending.len());
        for (id, pid) in pending {
            if !manager.is_live(&id) || manager.live_pid(&id) != pid {
                continue; // gone, or already re-exec'd by someone else
            }
            if manager.busy_for_restart(&id).await {
                still.push((id, pid));
                continue;
            }
            match manager.restart(&id, None).await {
                Ok(_) => info!(session = %id, "cli_auto_update: reloaded deferred session"),
                Err(e) => warn!(session = %id, "cli_auto_update: deferred reload failed: {e}"),
            }
            tokio::time::sleep(RESTART_STAGGER).await;
        }
        pending = still;
    }
    if !pending.is_empty() {
        info!(
            count = pending.len(),
            "cli_auto_update: gave up waiting for busy sessions; they keep the old binary until restarted"
        );
    }
}

fn json_now() -> serde_json::Value {
    serde_json::Value::String(Utc::now().to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_time() {
        assert_eq!(parse_hhmm("07:00"), Some((7, 0)));
        assert_eq!(parse_hhmm("23:59"), Some((23, 59)));
        assert_eq!(parse_hhmm("0:5"), Some((0, 5)));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("7"), None);
        assert_eq!(parse_hhmm("aa:bb"), None);
    }

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn not_due_before_scheduled_time() {
        // 02:30 UTC, scheduled 03:00 UTC, never ran → not yet.
        assert!(!is_due(utc(2026, 6, 20, 2, 30), true, 3, 0, None));
    }

    #[test]
    fn due_after_time_when_never_ran() {
        assert!(is_due(utc(2026, 6, 20, 3, 0), true, 3, 0, None));
        assert!(is_due(utc(2026, 6, 20, 9, 0), true, 3, 0, None));
    }

    #[test]
    fn not_due_when_already_ran_today() {
        // Ran at 03:05 UTC today; now 09:00 → already done for today's window.
        assert!(!is_due(
            utc(2026, 6, 20, 9, 0),
            true,
            3,
            0,
            Some(utc(2026, 6, 20, 3, 5))
        ));
    }

    #[test]
    fn catch_up_missed_window() {
        // Machine was off at 03:00 UTC; boots at 09:00, last ran yesterday → fire.
        assert!(is_due(
            utc(2026, 6, 20, 9, 0),
            true,
            3,
            0,
            Some(utc(2026, 6, 19, 3, 2))
        ));
    }

    #[test]
    fn local_tz_path_is_evaluated() {
        // Local mode: 12:00 local is well past a 03:00 local schedule → due.
        let now = Local.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
        assert!(is_due(now.with_timezone(&Utc), false, 3, 0, None));
    }

    #[test]
    fn config_defaults_on() {
        let c = CliAutoUpdateConfig::default();
        assert!(c.enabled);
        assert_eq!(c.time_of_day, "03:00");
        assert!(c.use_utc);
        assert!(c.reload_sessions);
    }

    #[test]
    fn config_partial_json_keeps_defaults() {
        let c: CliAutoUpdateConfig = serde_json::from_str(r#"{"enabled":false}"#).unwrap();
        assert!(!c.enabled);
        assert_eq!(c.time_of_day, "03:00"); // defaulted
        assert!(c.use_utc); // defaulted
        assert!(c.reload_sessions); // defaulted
    }

    /// An outcome whose probe could not read a version (counts as changed).
    fn outcome(name: &str, ok: bool, detail: &str) -> UpdateOutcome {
        UpdateOutcome {
            name: name.to_string(),
            ok,
            detail: detail.to_string(),
            before: None,
            after: None,
        }
    }

    fn versioned(name: &str, before: &str, after: &str) -> UpdateOutcome {
        UpdateOutcome {
            name: name.to_string(),
            ok: true,
            detail: String::new(),
            before: Some(before.to_string()),
            after: Some(after.to_string()),
        }
    }

    fn tally(reloaded: u32, failed: u32) -> Option<ReloadTally> {
        Some(ReloadTally {
            reloaded,
            failed,
            deferred: 0,
        })
    }

    #[test]
    fn exit_zero_without_a_version_change_is_not_an_update() {
        // r3-08-01: `claude update` exits 0 when already current; that must
        // not restart every live session.
        assert!(!versioned("claude", "2.1.0", "2.1.0").changed());
        assert!(versioned("claude", "2.1.0", "2.1.3").changed());
        // Unknown (probe failed) is treated as changed — the reload is still
        // deferred to idle, so this costs at most one quiet restart.
        assert!(version_changed(None, Some("1")));
        assert!(version_changed(Some("1"), None));
        // A failed updater never counts, whatever the probes say.
        assert!(!outcome("codex", false, "exit 1").changed());
    }

    #[test]
    fn body_separates_updated_from_already_current_and_counts_deferred() {
        let o = [
            versioned("claude", "2.1.0", "2.1.3"),
            versioned("codex", "0.9", "0.9"),
        ];
        assert_eq!(
            build_body(
                &o,
                Some(ReloadTally {
                    reloaded: 1,
                    failed: 0,
                    deferred: 2
                })
            ),
            "Updated CLIs: claude (2.1.0 → 2.1.3). Already up to date: codex. \
             Reloaded 1 open session(s); 2 busy session(s) reload when idle."
        );
        let same = [versioned("claude", "2.1.0", "2.1.0")];
        assert_eq!(
            build_body(&same, None),
            "No CLIs updated. Already up to date: claude."
        );
    }

    #[test]
    fn body_all_ok_lists_every_provider_as_updated() {
        let o = [outcome("agy", true, ""), outcome("claude", true, "")];
        assert_eq!(
            build_body(&o, tally(2, 0)),
            "Updated CLIs: agy, claude. Reloaded 2 open session(s)."
        );
    }

    #[test]
    fn body_one_failure_names_the_provider_and_reason() {
        // The regression this fixes: a codex config-parse failure must NOT be
        // reported as "Updated CLIs: … codex" with a dangling anonymous note.
        let o = [
            outcome("agy", true, ""),
            outcome("claude", true, ""),
            outcome(
                "codex",
                false,
                "exit 1: Error loading configuration: unknown variant `xhigh`",
            ),
        ];
        assert_eq!(
            build_body(&o, tally(1, 0)),
            "Updated CLIs: agy, claude. Failed: codex — exit 1: Error loading configuration: \
             unknown variant `xhigh`. Reloaded 1 open session(s)."
        );
    }

    #[test]
    fn body_all_failed_says_none_updated() {
        let o = [
            outcome("claude", false, "update timed out"),
            outcome("codex", false, "exit 7: boom"),
        ];
        assert_eq!(
            build_body(&o, None),
            "No CLIs updated. Failed: claude — update timed out; codex — exit 7: boom."
        );
    }

    #[test]
    fn body_reload_failures_are_counted() {
        let o = [outcome("claude", true, "")];
        assert_eq!(
            build_body(&o, tally(3, 2)),
            "Updated CLIs: claude. Reloaded 3 open session(s) (2 failed)."
        );
    }

    #[test]
    fn body_reload_disabled_omits_the_reload_sentence() {
        let o = [outcome("claude", true, "")];
        assert_eq!(build_body(&o, None), "Updated CLIs: claude.");
    }
}
