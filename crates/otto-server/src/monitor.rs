//! Credential monitor + session-event notices (wave 2).
//!
//! Independent producers, all funnelling through
//! [`ServerCtx::notifications`]'s de-duping `create()`:
//!
//! 1. [`CredentialMonitor`] — a background loop (startup, then every ~6h) that
//!    checks git/issue token expiry and agent-CLI credential health, emitting
//!    `Credential` notices.
//! 2. [`spawn_session_event_listener`] — subscribes to the event bus and, when
//!    `session_events` is enabled, emits `Session` notices on meaningful status
//!    transitions (idle / exited).
//! 3. [`AuthScanner`] — an [`OutputScanner`] wired into the `SessionManager`
//!    that scans live PTY output for re-auth prompts and emits an `Error`
//!    `Credential` notice (debounced once per session).
//! 4. [`spawn_budget_sampler`] — subscribes to [`Event::UsageMetricsTick`] and
//!    checks budgets on each tick. Emits `Event::BudgetExceeded` with
//!    `direction = "exceeded"` the first time a budget crosses its cap (and
//!    `"recovered"` once when it drops back below), so each window fires at most
//!    two events per `(scope, key)`. De-duplication is in-memory; keys that
//!    recover are removed from the alerted set so a future re-crossing fires again.
//!
//! Everything here is best-effort: read/probe errors are logged and skipped;
//! the loop never panics and never exits on a transient failure.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_core::domain::{
    GitAccount, GitProviderKind, IssueAccount, NoticeAction, NoticeKind, NoticeSeverity,
    SessionStatus, TaskStatus,
};
use otto_core::event::Event;
use otto_core::Id;
use otto_git::make_provider;
use otto_sessions::OutputScanner;
use otto_state::NewNotice;

use crate::state::ServerCtx;

/// Re-check cadence for the credential monitor.
const MONITOR_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

// ---------------------------------------------------------------------------
// Credential monitor loop
// ---------------------------------------------------------------------------

/// Background credential monitor. Spawn once at daemon start with
/// [`CredentialMonitor::spawn`].
pub struct CredentialMonitor {
    ctx: ServerCtx,
}

impl CredentialMonitor {
    pub fn new(ctx: ServerCtx) -> Self {
        Self { ctx }
    }

    /// Spawn the monitor loop: an immediate sweep, then one every
    /// [`MONITOR_INTERVAL`]. The task runs for the life of the process.
    pub fn spawn(self) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(MONITOR_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                self.sweep().await;
            }
        });
    }

    /// One full pass: git accounts, issue accounts, then agent CLIs. Each item
    /// is independent; a failure on one never aborts the sweep.
    async fn sweep(&self) {
        let threshold_days = match self.ctx.notifications().repo().get_settings().await {
            Ok(s) => i64::from(s.expiry_threshold_days),
            Err(e) => {
                tracing::warn!("credential monitor: read settings failed: {e}");
                3
            }
        };
        let now = Utc::now();

        self.check_git_accounts(now, threshold_days).await;
        self.check_issue_accounts(now, threshold_days).await;
        self.check_agent_clis().await;
    }

    // -- git accounts -------------------------------------------------------

    async fn check_git_accounts(&self, now: DateTime<Utc>, threshold_days: i64) {
        let accounts = match self.ctx.git_store.list_all_accounts().await {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("credential monitor: list git accounts: {e}");
                return;
            }
        };
        for account in accounts {
            let expiry = self.resolve_git_expiry(&account).await;
            let Some(expiry) = expiry else { continue };
            self.emit_expiry_notice(
                &format!("git_account:{}:expiry", account.id),
                &format!("git:{}", account.id),
                &account.label,
                provider_label(account.provider),
                expiry,
                now,
                threshold_days,
            )
            .await;
        }
    }

    /// Effective expiry for a git account: auto-detect for GitHub/GitLab (and
    /// persist it so the UI sees it), otherwise the stored value.
    async fn resolve_git_expiry(&self, account: &GitAccount) -> Option<DateTime<Utc>> {
        let auto_capable = matches!(
            account.provider,
            GitProviderKind::Github | GitProviderKind::Gitlab
        );
        if auto_capable {
            match self.secrets_token(&account.token_ref).await {
                Some(token) => {
                    let provider = make_provider(account, token);
                    match provider.token_expiry().await {
                        Ok(Some(detected)) => {
                            // Persist when it differs so it surfaces in the UI.
                            if account.token_expires_at != Some(detected) {
                                if let Err(e) = self
                                    .ctx
                                    .git_store
                                    .set_token_expiry(&account.id, Some(detected))
                                    .await
                                {
                                    tracing::warn!(
                                        "credential monitor: persist git expiry {}: {e}",
                                        account.id
                                    );
                                }
                            }
                            return Some(detected);
                        }
                        // Provider exposed no expiry → fall back to stored value.
                        Ok(None) => {}
                        Err(e) => {
                            tracing::debug!(
                                "credential monitor: git token probe {} failed: {e}",
                                account.id
                            );
                        }
                    }
                }
                None => {
                    tracing::debug!(
                        "credential monitor: git account {} has no stored token",
                        account.id
                    );
                }
            }
        }
        account.token_expires_at
    }

    // -- issue accounts -----------------------------------------------------

    async fn check_issue_accounts(&self, now: DateTime<Utc>, threshold_days: i64) {
        let accounts = match self.ctx.issues_store.list_all_accounts().await {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("credential monitor: list issue accounts: {e}");
                return;
            }
        };
        for account in accounts {
            // No auto-detect endpoint for Jira; rely on the stored value.
            let Some(expiry) = account.token_expires_at else {
                continue;
            };
            self.emit_expiry_notice(
                &format!("issue_account:{}:expiry", account.id),
                &format!("issue:{}", account.id),
                &account.label,
                issue_provider_label(&account),
                expiry,
                now,
                threshold_days,
            )
            .await;
        }
    }

    // -- shared expiry-notice emit ------------------------------------------

    #[allow(clippy::too_many_arguments)]
    async fn emit_expiry_notice(
        &self,
        source_key: &str,
        reauth_target: &str,
        label: &str,
        provider: &str,
        expiry: DateTime<Utc>,
        now: DateTime<Utc>,
        threshold_days: i64,
    ) {
        let days_left = (expiry - now).num_days();
        let expired = expiry <= now;
        let within_threshold = days_left <= threshold_days;
        if !expired && !within_threshold {
            return;
        }

        let (severity, title, body) = if expired {
            (
                NoticeSeverity::Error,
                format!("{provider} token expired"),
                format!("The token for \"{label}\" has expired. Re-authenticate to continue."),
            )
        } else {
            let when = if days_left <= 0 {
                "today".to_string()
            } else if days_left == 1 {
                "in 1 day".to_string()
            } else {
                format!("in {days_left} days")
            };
            (
                NoticeSeverity::Warn,
                format!("{provider} token expiring soon"),
                format!("The token for \"{label}\" expires {when}. Re-authenticate to avoid interruptions."),
            )
        };

        let _ = self
            .ctx
            .notifications()
            .create(NewNotice {
                kind: NoticeKind::Credential,
                severity,
                title,
                body,
                source_key: Some(source_key.to_string()),
                action: Some(NoticeAction::Reauth {
                    target: reauth_target.to_string(),
                }),
                user_id: None, // global credential notice
            })
            .await
            .map_err(|e| tracing::warn!("credential monitor: create expiry notice: {e}"));
    }

    // -- agent CLI health ---------------------------------------------------

    /// Presence-based health check for the local agent CLIs. We notify ONLY
    /// when credentials are absent/unusable — never on near access-expiry,
    /// because both CLIs auto-refresh their access tokens.
    async fn check_agent_clis(&self) {
        self.check_agent(
            "claude",
            // `security` subprocess: blocking pool, not a runtime worker.
            crate::offload::blocking(claude_credentials_present).await,
            "Claude: re-login needed",
            "Claude credentials are missing. Run `claude login` to re-authenticate.",
            "agent_auth:claude",
        )
        .await;

        self.check_agent(
            "codex",
            // File read: blocking pool too, not a runtime worker (S9-11).
            crate::offload::blocking(codex_credentials_present).await,
            "Codex: re-login needed",
            "Codex credentials are missing. Run `codex login` to re-authenticate.",
            "agent_auth:codex",
        )
        .await;
    }

    async fn check_agent(
        &self,
        target: &str,
        present: AgentHealth,
        title: &str,
        body: &str,
        source_key: &str,
    ) {
        match present {
            // Healthy or unknown (read error): stay silent. Spec says any read
            // error → skip silently; only emit on a definite "absent".
            AgentHealth::Healthy | AgentHealth::Unknown => {}
            AgentHealth::Missing => {
                let _ = self
                    .ctx
                    .notifications()
                    .create(NewNotice {
                        kind: NoticeKind::Credential,
                        severity: NoticeSeverity::Warn,
                        title: title.to_string(),
                        body: body.to_string(),
                        source_key: Some(source_key.to_string()),
                        action: Some(NoticeAction::Reauth {
                            target: target.to_string(),
                        }),
                        user_id: None, // global credential notice
                    })
                    .await
                    .map_err(|e| tracing::warn!("credential monitor: agent notice {target}: {e}"));
            }
        }
    }

    async fn secrets_token(&self, token_ref: &str) -> Option<String> {
        otto_core::secrets::get_async(&self.ctx.secrets, token_ref)
            .await
            .ok()
            .flatten()
    }
}

// ---------------------------------------------------------------------------
// Agent-CLI credential presence
// ---------------------------------------------------------------------------

/// Three-state health: explicitly present, explicitly absent, or unknown (read
/// error → treated as "skip" to avoid false alarms).
enum AgentHealth {
    Healthy,
    Missing,
    Unknown,
}

/// Claude stores its OAuth credentials in the macOS Keychain under the generic
/// password item service `Claude Code-credentials`. We only ever need to know
/// whether that item EXISTS — its decoded contents were never used (a present
/// but unparseable item was already treated as healthy).
///
/// Crucially we DON'T read the secret value: that pops a Keychain authorization
/// prompt ("ottod wants to use … Claude Code-credentials") on every fresh
/// install. ottod isn't on the item's ACL (the `claude` CLI created it), and
/// our self-signed dev cert's designated requirement changes each rebuild, so a
/// prior "Always Allow" never matches the new binary. `security
/// find-generic-password` WITHOUT `-w`/`-g` returns only item METADATA, which
/// needs no ACL approval and therefore never prompts. Found ⇒ healthy;
/// errSecItemNotFound (exit 44) ⇒ missing; anything else ⇒ unknown (skip, so we
/// never false-alarm on a transient error).
#[cfg(target_os = "macos")]
#[allow(clippy::disallowed_methods)] // sync helper: check_agent_clis runs it via offload::blocking
fn claude_credentials_present() -> AgentHealth {
    let status = std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", "Claude Code-credentials"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => AgentHealth::Healthy,
        Ok(s) if s.code() == Some(44) => AgentHealth::Missing,
        _ => AgentHealth::Unknown,
    }
}

#[cfg(not(target_os = "macos"))]
fn claude_credentials_present() -> AgentHealth {
    AgentHealth::Unknown
}

/// Codex stores credentials in `~/.codex/auth.json` (`tokens.access_token`
/// JWT + `last_refresh`). File present + parseable with a token ⇒ healthy;
/// file absent ⇒ missing; read/parse error ⇒ unknown.
fn codex_credentials_present() -> AgentHealth {
    let Some(home) = dirs::home_dir() else {
        return AgentHealth::Unknown;
    };
    let path: PathBuf = home.join(".codex").join("auth.json");
    match std::fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(v) => {
                let has_token = v
                    .get("tokens")
                    .and_then(|t| t.get("access_token"))
                    .and_then(|t| t.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                let has_api_key = v
                    .get("OPENAI_API_KEY")
                    .and_then(|t| t.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                if has_token || has_api_key {
                    AgentHealth::Healthy
                } else {
                    AgentHealth::Missing
                }
            }
            Err(_) => AgentHealth::Unknown,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => AgentHealth::Missing,
        Err(_) => AgentHealth::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Provider display labels
// ---------------------------------------------------------------------------

fn provider_label(p: GitProviderKind) -> &'static str {
    match p {
        GitProviderKind::Github => "GitHub",
        GitProviderKind::Gitlab => "GitLab",
        GitProviderKind::Bitbucket => "Bitbucket",
    }
}

fn issue_provider_label(_a: &IssueAccount) -> &'static str {
    "Jira"
}

// ---------------------------------------------------------------------------
// Session-progress notices (event-bus listener)
// ---------------------------------------------------------------------------

/// Subscribe to the event bus and record a usage row for every meaningful
/// activity-trail entry — the automatic side of usage tracking. Token counts /
/// model / cost are mined from each entry's `detail` when the provider reports
/// them (e.g. via `/ingest/usage`); otherwise the row still captures the action
/// as a per-provider/session/day activity count. The session's provider is
/// resolved once and cached. Cheap no-op while the engine is unavailable, so it
/// starts working the moment ClickHouse is installed (no restart needed).
pub fn spawn_usage_recorder(ctx: ServerCtx) {
    let mut rx = ctx.events.subscribe();
    tokio::spawn(async move {
        let mut providers: HashMap<Id, String> = HashMap::new();
        loop {
            match rx.recv().await {
                Ok(Event::TrailAppended {
                    workspace_id,
                    session_id,
                    event,
                }) => {
                    let provider = match providers.get(&session_id) {
                        Some(p) => p.clone(),
                        // A failed lookup (row not yet visible, DB busy) is
                        // NOT cached: the old `unwrap_or_default` pinned the
                        // empty provider on that session for its lifetime.
                        None => match ctx.manager.get(&session_id).await {
                            Ok(s) => {
                                providers.insert(session_id.clone(), s.provider.clone());
                                s.provider
                            }
                            Err(_) => String::new(),
                        },
                    };
                    if let Some(ev) = crate::routes::usage::trail_to_usage(
                        &workspace_id,
                        &session_id,
                        &provider,
                        &event,
                    ) {
                        ctx.usage.record(ev);
                    }
                }
                // Evict once the session can no longer emit trail rows from
                // its live child (exited / archived / removed); a resumed
                // session simply re-resolves on its next entry.
                Ok(Event::SessionRemoved { session_id, .. })
                | Ok(Event::SessionStatus {
                    session_id,
                    status: otto_core::domain::SessionStatus::Exited,
                    ..
                }) => {
                    providers.remove(&session_id);
                }
                Ok(Event::SessionArchiveChanged { session }) => {
                    providers.remove(&session.id);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Periodically sample host/process CPU + RAM into the usage engine's
/// `system_metrics` table. Re-reads the configured interval each tick so a
/// settings change takes effect without a restart. The sample itself is
/// blocking (it sleeps a CPU-refresh window), so it runs on a blocking thread.
///
/// Idle cost (R1a/R1b): a tick only samples while something needs it — a
/// live session, a recent `/usage/metrics` reader, or recently recorded usage
/// ([`otto_usage::UsageEngine::sampler_wanted`]); otherwise it skips the
/// sample AND the `UsageMetricsTick` broadcast (nothing changed for budgets
/// or sparklines to react to). Samples are buffered by the engine and
/// inserted in 5-minute batches. ONE `MetricsSampler` lives across ticks, so
/// the process CPU % is measured over the tick interval (a fresh sampler per
/// tick saw a single refresh and reported ~0).
pub fn spawn_metrics_sampler(ctx: ServerCtx) {
    tokio::spawn(async move {
        // A quick first sample so the dashboard has a data point seconds after
        // open, then sample on the configured cadence (re-read each loop so a
        // settings change takes effect within one interval).
        tokio::time::sleep(Duration::from_secs(3)).await;
        let sampler = std::sync::Arc::new(std::sync::Mutex::new(otto_usage::MetricsSampler::new()));
        loop {
            let live = ctx.manager.live_count();
            if ctx.usage.available() && ctx.usage.sampler_wanted(live) {
                let active = live as u32;
                let s = std::sync::Arc::clone(&sampler);
                match tokio::task::spawn_blocking(move || {
                    s.lock().unwrap_or_else(|p| p.into_inner()).sample(active)
                })
                .await
                {
                    Ok(metric) => {
                        if let Err(e) = ctx.usage.store_metric(&metric).await {
                            tracing::warn!("usage: store metric failed: {e}");
                        }
                        // Broadcast a tick so the dashboard can refresh
                        // sparklines in near-real-time without polling blindly.
                        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
                        let _ = ctx.events.send(Event::UsageMetricsTick { ts });
                    }
                    Err(e) => tracing::warn!("usage: metrics sampler join error: {e}"),
                }
            }
            tokio::time::sleep(ctx.usage.metrics_interval()).await;
        }
    });
}

/// Subscribe to the event bus and emit `Session` notices on meaningful status
/// transitions, gated by the `session_events` setting (re-read per event so a
/// settings change takes effect without restart). De-dupe is via the notice
/// `source_key`. Tracks the previous status per session in-memory so we only
/// fire on transitions into idle/exited (not every poll tick).
pub fn spawn_session_event_listener(ctx: ServerCtx) {
    let mut rx = ctx.events.subscribe();
    tokio::spawn(async move {
        let mut last: HashMap<Id, SessionStatus> = HashMap::new();
        loop {
            match rx.recv().await {
                Ok(Event::SessionStatus {
                    session_id, status, ..
                }) => {
                    let prev = last.insert(session_id.clone(), status);
                    if prev == Some(status) {
                        continue; // no real transition
                    }
                    if matches!(status, SessionStatus::Exited) {
                        last.remove(&session_id);
                    }
                    handle_session_transition(&ctx, &session_id, prev, status).await;
                }
                Ok(Event::SessionRemoved { session_id, .. }) => {
                    last.remove(&session_id);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Session `meta.source` values that mark BACKGROUND sessions — spawned by a
/// scheduler / runner, not driven by the user. They idle and end constantly by
/// design, so "awaiting input"/"ended" notices for them are pure noise (a
/// scheduled Insights run nagging "Claude is waiting for your input" was the
/// canonical false alarm). Mirrors the UI's sidebar blacklist.
const BACKGROUND_SOURCES: &[&str] = &[
    "channel",
    "insights",
    "workflow",
    "review",
    "skilleval",
    "skillreview",
    "product-analysis",
    "swarm",
    "canvas_assist",
    "mockup_assist",
    "db_assist",
    "browser_summarize",
    // Assistant replies surface as `assistant_turn` events + the needs-you
    // queue; a "waiting for your input" notice per reply would be noise.
    "assistant",
];

/// True when the session was spawned by a background runner (see
/// [`BACKGROUND_SOURCES`]) — such sessions never produce session notices.
/// Shared with the activity ingest (Claude's Notification hook), which fires
/// "Agent needs attention" and must stay quiet for background runs too.
pub(crate) fn is_background(s: &otto_core::domain::Session) -> bool {
    s.meta
        .get("source")
        .and_then(|v| v.as_str())
        .is_some_and(|src| BACKGROUND_SOURCES.contains(&src))
}

/// How long a session must STAY idle before the "awaiting input" notice fires.
/// The raw status flips to Idle after only ~5s of PTY silence — an agent
/// mid-thought or inside a quiet tool call trips that constantly, so the idle
/// transition alone is far too sensitive a "needs you" signal. The notice is
/// emitted only after the session is re-checked and found still idle AND its
/// process tree is not actively burning CPU (a running build/test/deploy).
/// Tunable via the `idle_notice_confirm_secs` setting.
const IDLE_CONFIRM: Duration = Duration::from_secs(45);

/// Read the confirm window from settings (`idle_notice_confirm_secs`), falling
/// back to [`IDLE_CONFIRM`]. Read per event so changes apply live.
async fn idle_confirm(ctx: &ServerCtx) -> Duration {
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    match repo.get("idle_notice_confirm_secs").await {
        Ok(Some(v)) => v.as_u64().map(Duration::from_secs).unwrap_or(IDLE_CONFIRM),
        _ => IDLE_CONFIRM,
    }
}

async fn handle_session_transition(
    ctx: &ServerCtx,
    session_id: &Id,
    prev: Option<SessionStatus>,
    status: SessionStatus,
) {
    // Only notify for transitions out of an active state into idle/exited.
    let was_active = matches!(
        prev,
        Some(SessionStatus::Working) | Some(SessionStatus::Running)
    );

    match status {
        SessionStatus::Idle if was_active => {
            // Debounce: confirm the idle sticks before telling the user their
            // agent is waiting. A session that resumes working (or exits) within
            // the window fires nothing — the exit path has its own notice.
            let ctx = ctx.clone();
            let id = session_id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(idle_confirm(&ctx).await).await;
                let Ok(s) = ctx.manager.get(&id).await else {
                    return;
                };
                if s.status != SessionStatus::Idle {
                    return; // resumed / exited meanwhile — false alarm avoided
                }
                // Quiet PTY but a busy process tree = a command is still
                // running under the agent (build / tests / deploy) — it does
                // NOT need the user yet. The eventual resume produces a fresh
                // Working→Idle transition, so the notice isn't lost, just late.
                // Cheap gates FIRST (S9-07): background sessions and a
                // disabled `session_events` setting discard the notice anyway,
                // so they must not pay `tree_active`'s two whole-machine `ps`
                // scans + 750 ms for it. `emit_session_notice` re-checks both.
                if !idle_notice_wanted(&ctx, &s).await {
                    return;
                }
                if ctx.manager.tree_active(&id).await {
                    return;
                }
                emit_session_notice(&ctx, &id, s, SessionStatus::Idle).await;
            });
        }
        SessionStatus::Exited => {
            let Ok(s) = ctx.manager.get(session_id).await else {
                return;
            };
            emit_session_notice(ctx, session_id, s, SessionStatus::Exited).await;
        }
        _ => {}
    }
}

/// Whether an idle notice for `s` could be emitted at all: not a background
/// session and session notices enabled. Checked before the expensive
/// process-tree probe.
async fn idle_notice_wanted(ctx: &ServerCtx, s: &otto_core::domain::Session) -> bool {
    if is_background(s) {
        return false;
    }
    matches!(
        ctx.notifications().repo().get_settings().await,
        Ok(settings) if settings.session_events
    )
}

/// Build + create the idle/exited notice for a (already re-validated) session.
async fn emit_session_notice(
    ctx: &ServerCtx,
    session_id: &Id,
    s: otto_core::domain::Session,
    status: SessionStatus,
) {
    let (severity, title, suffix) = match status {
        SessionStatus::Idle => (NoticeSeverity::Info, "Session awaiting input", "idle"),
        _ => (NoticeSeverity::Info, "Session ended", "exited"),
    };

    // Background sessions (channels, insights, workflow steps, reviews …) idle
    // and end by design — never notify for them.
    if is_background(&s) {
        return;
    }

    // Provider-agnostic "needs you": for AGENT sessions, a Working→Idle
    // transition is the turn finishing — the agent is now awaiting input. Use
    // the `:waiting` source_key suffix so the UI raises its sticky needs-you
    // flag (events.svelte.ts keys off `:waiting` + open_session), exactly like
    // claude's native Notification hook — this extends the same signal to
    // codex/agy/custom providers. Shell sessions keep the plain `:idle` key
    // (every command ending would otherwise light up the wall).
    let suffix = if suffix == "idle" && s.kind == otto_core::domain::SessionKind::Agent {
        "waiting"
    } else {
        suffix
    };

    // Build an informative body: "«title» (provider)" + the current task, if any.
    // For idle, the in-progress task is the most useful "what it was on" hint.
    let label = format!("{} ({})", s.title, s.provider);
    let current_task = ctx
        .activity()
        .repo()
        .list_tasks(session_id)
        .await
        .ok()
        .and_then(|tasks| {
            tasks
                .into_iter()
                .find(|t| t.status == TaskStatus::InProgress)
                .map(|t| t.title)
        });
    let body = match status {
        SessionStatus::Idle => match &current_task {
            Some(task) => {
                format!("{label} is idle and may be waiting for your input · was on: {task}")
            }
            None => format!("{label} is idle and may be waiting for your input."),
        },
        _ => match &current_task {
            Some(task) => format!("{label} has ended · last task: {task}"),
            None => format!("{label} has ended."),
        },
    };
    let title = title.to_string();

    // Re-read the gate per event so toggling the setting takes effect live.
    match ctx.notifications().repo().get_settings().await {
        Ok(s) if !s.session_events => return,
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("session events: read settings: {e}");
            return;
        }
    }

    let _ = ctx
        .notifications()
        .create(NewNotice {
            kind: NoticeKind::Session,
            severity,
            title,
            body,
            source_key: Some(format!("session:{session_id}:{suffix}")),
            action: Some(NoticeAction::OpenSession {
                session_id: session_id.clone(),
            }),
            user_id: None, // global session notice
        })
        .await
        .map_err(|e| tracing::warn!("session events: create notice: {e}"));
}

// ---------------------------------------------------------------------------
// Mid-session re-auth detection (PTY output scanner)
// ---------------------------------------------------------------------------

/// Substrings (lower-cased) that signal an agent CLI is demanding re-auth.
const REAUTH_NEEDLES: &[&str] = &[
    "run `claude login`",
    "run 'claude login'",
    "claude login",
    "codex login",
    "run `codex login`",
    "authentication required",
    "session expired",
    "please sign in",
    "please log in",
    "please login",
    "sign in to continue",
    "you are not logged in",
    "not authenticated",
];

/// Scans live PTY output for re-auth prompts and raises an `Error` `Credential`
/// notice, debounced once per session. Wired into the `SessionManager` via
/// [`otto_sessions::SessionManager::with_output_scanner`].
///
/// Holds a [`NotificationService`] (not the full `ServerCtx`) so it can be
/// attached to the `SessionManager` *before* `ServerCtx` is assembled.
pub struct AuthScanner {
    notifications: crate::state::NotificationService,
    /// Sessions already flagged this lifetime (debounce). The `source_key`
    /// de-dupe + this set both guard against spam.
    flagged: Mutex<std::collections::HashSet<Id>>,
    /// Per-session rolling tail of recent output (needles can straddle chunk
    /// boundaries). Bounded to the last few hundred bytes.
    tails: Mutex<HashMap<Id, Vec<u8>>>,
}

impl AuthScanner {
    /// Build from a DB pool + event bus (available before `ServerCtx`).
    pub fn new(
        pool: otto_state::DbPool,
        events: tokio::sync::broadcast::Sender<Event>,
    ) -> Arc<Self> {
        Arc::new(Self {
            notifications: crate::state::NotificationService::new(pool, events),
            flagged: Mutex::new(std::collections::HashSet::new()),
            tails: Mutex::new(HashMap::new()),
        })
    }
}

/// Max retained tail bytes per session (covers the longest needle + slack).
const TAIL_CAP: usize = 256;

impl OutputScanner for AuthScanner {
    fn on_output(&self, session_id: &Id, provider: &str, chunk: &[u8]) {
        // A plain shell is not an agent CLI with a login to renew, and its
        // arbitrary output (curl/git/ssh "not authenticated", …) only ever
        // produced false re-auth alerts — skip the scan (perf 01 F6).
        if provider == "shell" {
            return;
        }
        // Already flagged this session → nothing to do.
        {
            let flagged = match self.flagged.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if flagged.contains(session_id) {
                return;
            }
        }

        // Search the rolling tail + the WHOLE chunk, then keep a short tail
        // (perf 01 F6: the old append-trim-search order cut a phrase early in
        // a large chunk off before looking, and lowercased bytes it dropped).
        let hit = {
            let mut tails = match self.tails.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let buf = tails.entry(session_id.clone()).or_default();
            otto_sessions::tail_scan::scan_chunk(buf, chunk, REAUTH_NEEDLES, TAIL_CAP)
        };
        if hit.is_none() {
            return;
        }

        // Mark flagged (race-safe: re-check inside the lock).
        {
            let mut flagged = match self.flagged.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if !flagged.insert(session_id.clone()) {
                return;
            }
        }
        // Drop the tail now that we've fired.
        if let Ok(mut tails) = self.tails.lock() {
            tails.remove(session_id);
        }

        let notifications = self.notifications.clone();
        let session_id = session_id.clone();
        let provider = provider.to_string();
        tokio::spawn(async move {
            let display = match provider.as_str() {
                "claude" => "Claude",
                "codex" => "Codex",
                other => other,
            };
            let _ = notifications
                .create(NewNotice {
                    kind: NoticeKind::Credential,
                    severity: NoticeSeverity::Error,
                    title: format!("{display}: re-authentication required"),
                    body: format!(
                        "An agent session detected a re-authentication prompt. Re-authenticate {display} to continue."
                    ),
                    source_key: Some(format!("session_auth:{session_id}")),
                    action: Some(NoticeAction::Reauth { target: provider }),
                    user_id: None, // global credential notice
                })
                .await
                .map_err(|e| tracing::warn!("mid-session auth notice: {e}"));
        });
    }

    /// The session's output stream closed: forget its tail and debounce mark.
    /// Both maps were insert-only before, so every session the daemon ever ran
    /// kept up to `TAIL_CAP` bytes (+ its id) for the process lifetime. A
    /// respawn (same id) scans afresh; the notice's `source_key` still
    /// de-dupes a repeat alert.
    fn on_session_end(&self, session_id: &Id) {
        match self.tails.lock() {
            Ok(mut g) => g.remove(session_id),
            Err(p) => p.into_inner().remove(session_id),
        };
        match self.flagged.lock() {
            Ok(mut g) => g.remove(session_id),
            Err(p) => p.into_inner().remove(session_id),
        };
    }
}

// ---------------------------------------------------------------------------
// Budget sampler (rides the metrics tick)
// ---------------------------------------------------------------------------

/// Subscribe to `UsageMetricsTick` and check configured budgets on each tick.
///
/// When a budget with `enforce = true` has `spent >= cap` and it was not
/// already in the alerted set, emit `Event::BudgetExceeded` with
/// `direction = "exceeded"` and add the key to the set. When a key that was
/// alerted drops back below the cap, emit `direction = "recovered"` and remove
/// it from the set so a future re-crossing fires again.
///
/// De-dupe is purely in-memory; a daemon restart resets the set, which is
/// acceptable — the UI dismisses banners on its own and re-alerting after a
/// restart is harmless.
pub fn spawn_budget_sampler(ctx: ServerCtx) {
    let mut rx = ctx.events.subscribe();
    tokio::spawn(async move {
        // De-duplicator: emits Exceeded once per crossing, Recovered once on
        // drop-back. Implemented in otto-usage::BudgetDedup (unit-tested there).
        let mut dedup = otto_usage::BudgetDedup::new();
        loop {
            match rx.recv().await {
                Ok(Event::UsageMetricsTick { .. }) => {
                    check_budgets(&ctx, &mut dedup).await;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// One budget-check pass: load config + current spend, then compare each row
/// against the de-duplication state and emit `BudgetExceeded` as needed.
async fn check_budgets(ctx: &ServerCtx, dedup: &mut otto_usage::BudgetDedup) {
    let cfg = crate::routes::usage::load_budgets_pub(ctx).await;
    if !cfg.enforce {
        // Enforcement is off — clear any stale crossing state so that the next
        // time enforcement is turned back on we start fresh.
        dedup.clear();
        return;
    }
    // Spend only moves when usage is written (or the window rolls / config
    // changes): skip the scan otherwise, so a metrics tick never wakes an
    // idle-stopped ClickHouse (perf3 N1). Stamped BEFORE the scan.
    let stamp = otto_usage::BudgetCheckStamp::now(
        ctx.usage.usage_generation(),
        serde_json::to_string(&cfg).unwrap_or_default(),
    );
    if dedup.unchanged_since_last_check(&stamp) {
        return;
    }
    // Background read (S9-303): never resets ClickHouse's idle clock (each
    // flush bumps the generation, so a stamping scan here kept an always-on
    // agent's server up forever) and never wakes a parked one — no spend to
    // judge yet means "check again next tick", so the stamp is not marked.
    let Some(status) = crate::routes::usage::budget_status_background(ctx, cfg).await else {
        return;
    };
    dedup.mark_checked(stamp);
    for row in &status.rows {
        let signal = dedup.apply(&row.scope, &row.key, row.exceeded);
        let direction = match signal {
            otto_usage::BudgetSignal::Exceeded => "exceeded",
            otto_usage::BudgetSignal::Recovered => "recovered",
            otto_usage::BudgetSignal::NoChange => continue,
        };
        let _ = ctx.events.send(Event::BudgetExceeded {
            workspace_id: if row.scope == "workspace" {
                row.key.clone()
            } else {
                String::new()
            },
            provider: if row.scope == "provider" {
                row.key.clone()
            } else {
                String::new()
            },
            spend_usd: row.spent_usd,
            cap_usd: row.limit_usd,
            direction: direction.to_string(),
        });
        tracing::info!(
            scope = %row.scope,
            key = %row.key,
            spent = row.spent_usd,
            cap = row.limit_usd,
            direction,
            "budget crossing — BudgetExceeded emitted"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{REAUTH_NEEDLES, TAIL_CAP};
    use otto_sessions::tail_scan::scan_chunk;

    /// Regression: the rolling tail must never panic when the cut point lands
    /// inside a multi-byte glyph (the Powerline separator U+E0B0 is 3 bytes);
    /// the byte tail stays bounded and a later needle is still found.
    #[test]
    fn tail_survives_multibyte_glyphs() {
        let mut tail = Vec::new();
        let glyphs = "\u{e0b0}".repeat(200);
        assert_eq!(
            scan_chunk(&mut tail, glyphs.as_bytes(), REAUTH_NEEDLES, TAIL_CAP),
            None
        );
        assert!(tail.len() <= TAIL_CAP);
        assert!(scan_chunk(
            &mut tail,
            b"Please Sign In to continue",
            REAUTH_NEEDLES,
            TAIL_CAP
        )
        .is_some());
    }

    /// Perf 01 F6: a re-auth line at the START of a large chunk was trimmed
    /// away before the search (append → trim to 256 → search).
    #[test]
    fn reauth_line_early_in_a_large_chunk_is_detected() {
        let mut chunk = b"Session expired. Run `claude login`\n".to_vec();
        chunk.extend(std::iter::repeat_n(b'.', 8000));
        let mut tail = Vec::new();
        assert!(scan_chunk(&mut tail, &chunk, REAUTH_NEEDLES, TAIL_CAP).is_some());
        assert!(tail.len() <= TAIL_CAP, "only a short tail is kept");
    }

    /// A needle split across two chunks is still found from the kept tail.
    #[test]
    fn reauth_line_split_across_chunks_is_detected() {
        let mut tail = Vec::new();
        let mut first = vec![b'x'; 5000];
        first.extend_from_slice(b"you are not log");
        assert_eq!(
            scan_chunk(&mut tail, &first, REAUTH_NEEDLES, TAIL_CAP),
            None
        );
        assert_eq!(
            scan_chunk(&mut tail, b"ged in\n", REAUTH_NEEDLES, TAIL_CAP),
            Some("you are not logged in")
        );
    }

    /// Perf P2: per-session scanner state is released when the stream ends —
    /// both the rolling tail and the debounce mark (they used to grow with
    /// every session for the daemon's lifetime).
    #[tokio::test]
    async fn session_end_releases_tail_and_flag() {
        use otto_sessions::OutputScanner;
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("t.db")).await.unwrap();
        let (tx, _rx) = tokio::sync::broadcast::channel(8);
        let scanner = super::AuthScanner::new(pool, tx);
        let (quiet, flagged) = ("s-quiet".to_string(), "s-flagged".to_string());
        scanner.on_output(&quiet, "claude", b"working on it, you are not log");
        scanner.on_output(&flagged, "claude", b"Session expired. Run `claude login`");
        assert!(scanner.tails.lock().unwrap().contains_key(&quiet));
        assert!(scanner.flagged.lock().unwrap().contains(&flagged));
        scanner.on_session_end(&quiet);
        scanner.on_session_end(&flagged);
        assert!(scanner.tails.lock().unwrap().is_empty());
        assert!(scanner.flagged.lock().unwrap().is_empty());
    }
}
