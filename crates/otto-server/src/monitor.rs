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
            match self.secrets_token(&account.token_ref) {
                Some(token) => {
                    let provider = make_provider(account, token);
                    match provider.token_expiry().await {
                        Ok(Some(detected)) => {
                            // Persist when it differs so it surfaces in the UI.
                            if account.token_expires_at != Some(detected) {
                                if let Err(e) =
                                    self.ctx.git_store.set_token_expiry(&account.id, Some(detected)).await
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
            claude_credentials_present(),
            "Claude: re-login needed",
            "Claude credentials are missing. Run `claude login` to re-authenticate.",
            "agent_auth:claude",
        )
        .await;

        self.check_agent(
            "codex",
            codex_credentials_present(),
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

    fn secrets_token(&self, token_ref: &str) -> Option<String> {
        self.ctx.secrets.get(token_ref).ok().flatten()
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
                        None => {
                            let p = ctx
                                .manager
                                .get(&session_id)
                                .await
                                .map(|s| s.provider)
                                .unwrap_or_default();
                            providers.insert(session_id.clone(), p.clone());
                            p
                        }
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
                Ok(Event::SessionRemoved { session_id, .. }) => {
                    providers.remove(&session_id);
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
pub fn spawn_metrics_sampler(ctx: ServerCtx) {
    tokio::spawn(async move {
        // A quick first sample so the dashboard has a data point seconds after
        // open, then sample on the configured cadence (re-read each loop so a
        // settings change takes effect within one interval).
        tokio::time::sleep(Duration::from_secs(3)).await;
        loop {
            if ctx.usage.available() {
                let active = ctx.manager.live_count() as u32;
                match tokio::task::spawn_blocking(move || {
                    otto_usage::MetricsSampler::new().sample(active)
                })
                .await
                {
                    Ok(metric) => {
                        if let Err(e) = ctx.usage.store_metric(&metric).await {
                            tracing::warn!("usage: store metric failed: {e}");
                        }
                        // Broadcast a tick so the dashboard can refresh
                        // sparklines in near-real-time without polling blindly.
                        let ts = chrono::Utc::now()
                            .format("%Y-%m-%dT%H:%M:%SZ")
                            .to_string();
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
                    session_id,
                    status,
                    ..
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

    let kind = match status {
        SessionStatus::Idle if was_active => {
            Some((NoticeSeverity::Info, "Session awaiting input", "idle"))
        }
        SessionStatus::Exited => Some((NoticeSeverity::Info, "Session ended", "exited")),
        _ => None,
    };
    let Some((severity, title, suffix)) = kind else {
        return;
    };

    // Load the session so the notice can name it (title + provider + what it was
    // doing) instead of a bare "an agent session". Bail if it's gone.
    let Ok(s) = ctx.manager.get(session_id).await else {
        return;
    };

    // Background channel (Slack/Telegram) sessions end after every reply — they
    // would flood the notification center, so never notify for them.
    if s.meta.get("source").and_then(|v| v.as_str()) == Some("channel") {
        return;
    }

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
            Some(task) => format!("{label} is idle and may be waiting for your input · was on: {task}"),
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
    tails: Mutex<HashMap<Id, String>>,
}

impl AuthScanner {
    /// Build from a DB pool + event bus (available before `ServerCtx`).
    pub fn new(
        pool: sqlx::SqlitePool,
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

/// Trim `buf` in place to at most `cap` bytes, keeping the most-recent content and
/// NEVER splitting a UTF-8 code point. Terminal output routinely contains
/// multi-byte glyphs (e.g. the Powerline prompt separator U+E0B0 ``), so a naive
/// `buf[buf.len() - cap..]` byte slice can land mid-char and panic the worker
/// thread. We advance the cut forward to the next char boundary instead — keeping
/// ≤ `cap` bytes, which still comfortably covers the longest needle.
fn trim_tail(buf: &mut String, cap: usize) {
    if buf.len() <= cap {
        return;
    }
    let mut cut = buf.len() - cap;
    while cut < buf.len() && !buf.is_char_boundary(cut) {
        cut += 1;
    }
    buf.replace_range(..cut, "");
}

impl OutputScanner for AuthScanner {
    fn on_output(&self, session_id: &Id, provider: &str, chunk: &[u8]) {
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

        // Append to the rolling tail and search the combined window.
        let text = String::from_utf8_lossy(chunk).to_lowercase();
        let combined = {
            let mut tails = match self.tails.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let buf = tails.entry(session_id.clone()).or_default();
            buf.push_str(&text);
            trim_tail(buf, TAIL_CAP);
            buf.clone()
        };

        if !REAUTH_NEEDLES.iter().any(|n| combined.contains(n)) {
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
    let status = crate::routes::usage::budget_status_pub(ctx, cfg).await;
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
    use super::trim_tail;

    /// Regression: the rolling tail must never panic when the cut point lands
    /// inside a multi-byte glyph. The Powerline separator U+E0B0 (`\u{e0b0}`) is
    /// 3 bytes, so a buffer of them has cut points that are NOT char boundaries.
    #[test]
    fn trim_tail_handles_multibyte_glyphs() {
        let glyph = '\u{e0b0}';
        let mut s = String::new();
        for _ in 0..200 {
            s.push(glyph); // 600 bytes — well over the cap
        }
        trim_tail(&mut s, 256); // must not panic on a mid-char byte index
        assert!(s.len() <= 256, "tail trimmed to within the cap");
        assert!(s.chars().all(|c| c == glyph), "no split/garbled code points");
    }

    #[test]
    fn trim_tail_is_a_noop_below_cap() {
        let mut s = "needs reauthentication".to_string();
        trim_tail(&mut s, 256);
        assert_eq!(s, "needs reauthentication");
    }

    #[test]
    fn trim_tail_keeps_the_most_recent_bytes() {
        // 1000 ASCII bytes; trimming must retain the *newest* tail (a needle that
        // just arrived must survive), not the oldest.
        let mut s: String = (0..1000).map(|i| (b'a' + (i % 26) as u8) as char).collect();
        let want_tail: String = s.chars().rev().take(50).collect::<Vec<_>>().into_iter().rev().collect();
        trim_tail(&mut s, 256);
        assert!(s.len() <= 256);
        assert!(s.ends_with(&want_tail), "kept the most recent content");
    }
}

