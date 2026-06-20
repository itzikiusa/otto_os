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
fn scheduled_today(now: DateTime<Utc>, use_utc: bool, hour: u32, minute: u32) -> Option<DateTime<Utc>> {
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
    cancel: Arc<AtomicBool>,
    _supervisor: JoinHandle<()>,
}

impl CliUpdateSchedulerHandle {
    pub fn shutdown(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for CliUpdateSchedulerHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
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
        let cancel = Arc::new(AtomicBool::new(false));
        let supervisor = tokio::spawn(self.supervise(Arc::clone(&cancel)));
        CliUpdateSchedulerHandle {
            cancel,
            _supervisor: supervisor,
        }
    }

    async fn supervise(self, cancel: Arc<AtomicBool>) {
        // Single in-flight guard: a run takes minutes; never overlap.
        let running = Arc::new(AtomicBool::new(false));
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            if !running.load(Ordering::Relaxed) {
                self.tick(&running).await;
            }
            // Sleep in short slices for responsive shutdown.
            let mut waited = Duration::ZERO;
            while waited < TICK {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                waited += Duration::from_millis(500);
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

/// Run one update pass. Public so a future "run now" route can reuse it.
pub async fn run(ctx: &ServerCtx, cfg: &CliAutoUpdateConfig) {
    let settings = SettingsRepo::new(ctx.pool.clone());

    let pairs = ctx.manager.provider_update_commands();
    if pairs.is_empty() {
        info!("cli_auto_update: no providers have an update command; nothing to do");
        let _ = settings.put(LAST_RUN_KEY, &json_now()).await;
        return;
    }
    let provider_names: HashSet<String> = pairs.iter().map(|(n, _)| n.clone()).collect();
    let label = {
        let mut v: Vec<&str> = pairs.iter().map(|(n, _)| n.as_str()).collect();
        v.sort_unstable();
        v.join(", ")
    };
    // Join steps so each provider's output is separated by a blank line, then run
    // through a login shell so the user's PATH/profile resolves every CLI.
    let compound = pairs
        .iter()
        .map(|(_, cmd)| cmd.as_str())
        .collect::<Vec<_>>()
        .join("; echo; ");

    let (update_ok, detail) = run_updates(&compound).await;
    info!(ok = update_ok, providers = %label, "cli_auto_update: updates finished");

    // Reload open agent sessions whose provider was just updated.
    let (reloaded, reload_failed) = if cfg.reload_sessions {
        reload_agent_sessions(ctx, &provider_names).await
    } else {
        (0, 0)
    };

    let _ = settings.put(LAST_RUN_KEY, &json_now()).await;

    // Summary notice (system-wide, de-duped by source key).
    let severity = if update_ok && reload_failed == 0 {
        NoticeSeverity::Info
    } else {
        NoticeSeverity::Warn
    };
    let mut body = format!("Updated CLIs: {label}.");
    if cfg.reload_sessions {
        body.push_str(&format!(" Reloaded {reloaded} open session(s)"));
        if reload_failed > 0 {
            body.push_str(&format!(" ({reload_failed} failed)"));
        }
        body.push('.');
    }
    if !update_ok {
        body.push_str(&format!(" Note: {detail}"));
    }
    let _ = ctx
        .notifications()
        .create(NewNotice {
            kind: NoticeKind::System,
            severity,
            title: "Daily CLI update".to_string(),
            body,
            source_key: Some(NOTICE_KEY.to_string()),
            action: None,
            user_id: None,
        })
        .await;
}

/// Run the compound update command via a login shell, bounded by a timeout.
/// Returns `(success, short_detail)`.
async fn run_updates(compound: &str) -> (bool, String) {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let fut = tokio::process::Command::new(&shell)
        .arg("-l")
        .arg("-c")
        .arg(compound)
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
/// (ssh/db) and plain shells are left alone. `restart` auto-resumes.
async fn reload_agent_sessions(ctx: &ServerCtx, providers: &HashSet<String>) -> (u32, u32) {
    let (mut ok, mut failed) = (0u32, 0u32);
    let workspaces = match ctx.workspaces.list_all().await {
        Ok(w) => w,
        Err(e) => {
            warn!("cli_auto_update: list workspaces failed: {e}");
            return (0, 0);
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
            match ctx.manager.restart(&s.id, None).await {
                Ok(_) => {
                    ok += 1;
                    info!(session = %s.id, provider = %s.provider, "cli_auto_update: reloaded session");
                }
                Err(e) => {
                    failed += 1;
                    warn!(session = %s.id, "cli_auto_update: reload failed: {e}");
                }
            }
        }
    }
    (ok, failed)
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
}
