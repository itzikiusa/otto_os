//! Credential monitor + session-event notices (wave 2).
//!
//! Three independent producers, all funnelling through
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
    SessionStatus,
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
/// password item service `Claude Code-credentials`, account = the login name.
/// Present + parseable `claudeAiOauth` ⇒ healthy; absent item ⇒ missing; any
/// other error ⇒ unknown.
fn claude_credentials_present() -> AgentHealth {
    let account = std::env::var("USER").unwrap_or_default();
    let entry = match keyring::Entry::new("Claude Code-credentials", &account) {
        Ok(e) => e,
        Err(_) => return AgentHealth::Unknown,
    };
    match entry.get_password() {
        Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(v) if v.get("claudeAiOauth").is_some() => AgentHealth::Healthy,
            // Item exists but is unparseable / wrong shape → treat as present
            // (don't nag); only a truly absent item is "missing".
            _ => AgentHealth::Healthy,
        },
        Err(keyring::Error::NoEntry) => AgentHealth::Missing,
        Err(_) => AgentHealth::Unknown,
    }
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

    let notice = match status {
        SessionStatus::Idle if was_active => Some((
            NoticeSeverity::Info,
            "Session awaiting input".to_string(),
            "An agent session is idle and may be waiting for your input.".to_string(),
            "idle",
        )),
        SessionStatus::Exited => Some((
            NoticeSeverity::Info,
            "Session ended".to_string(),
            "An agent session has ended.".to_string(),
            "exited",
        )),
        _ => None,
    };
    let Some((severity, title, body, suffix)) = notice else {
        return;
    };

    // Background channel (Slack/Telegram) sessions end after every reply — they
    // would flood the notification center, so never notify for them.
    if let Ok(s) = ctx.manager.get(session_id).await {
        if s.meta.get("source").and_then(|v| v.as_str()) == Some("channel") {
            return;
        }
    }

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
            if buf.len() > TAIL_CAP {
                let cut = buf.len() - TAIL_CAP;
                *buf = buf[cut..].to_string();
            }
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
                })
                .await
                .map_err(|e| tracing::warn!("mid-session auth notice: {e}"));
        });
    }
}

