//! How a thread's last finished turn ended, on its list row
//! ([`RuntimeSession::last_turn_status`] / [`RuntimeSession::last_error_class`]),
//! so a sidebar of many threads can tell "done" from "broke 20 minutes ago"
//! without loading every projection. Derived, never stored on its own: the
//! runtime keeps it from the live projection, the store from the persisted
//! latest `turn` entity (and its `<turn>:error` item).

use serde::{Deserialize, Serialize};

use super::{ProviderFailure, ProviderTurn, ProviderTurnStatus, RuntimeSession};

/// The outcome of a finished turn (`cancelled` reads as `interrupted`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    Failed,
    Interrupted,
}

impl TurnOutcome {
    /// `None` while the turn has not ended (pending / running).
    pub fn of(status: ProviderTurnStatus) -> Option<Self> {
        match status {
            ProviderTurnStatus::Completed => Some(Self::Completed),
            ProviderTurnStatus::Failed => Some(Self::Failed),
            ProviderTurnStatus::Interrupted | ProviderTurnStatus::Cancelled => {
                Some(Self::Interrupted)
            }
            ProviderTurnStatus::Pending | ProviderTurnStatus::Running => None,
        }
    }
}

/// The provider's plan / rate limit or quota ran out.
pub const ERROR_CLASS_USAGE_LIMIT: &str = "usage_limit";
/// Not signed in, an expired login or a bad key.
pub const ERROR_CLASS_AUTH: &str = "auth";
/// Anything else the agent CLI or its protocol reported.
pub const ERROR_CLASS_PROVIDER: &str = "provider";

/// Structured codes the drivers carry ([`ProviderFailure::code`]): Codex's
/// `codexErrorInfo` variant, Claude's assistant `error`, ACP's `-32000`
/// (`auth_required`). A code listed here decides; the text is not read.
const USAGE_CODES: &[&str] = &[
    "usageLimitExceeded",
    "usage_limit",
    "rate_limit",
    "rate_limit_error",
    "billing_error",
    "insufficient_quota",
];
const AUTH_CODES: &[&str] = &[
    "unauthorized",
    "authentication_failed",
    "authentication_error",
    "auth_required",
    "-32000",
];
const PROVIDER_CODES: &[&str] = &[
    "contextWindowExceeded",
    "httpConnectionFailed",
    "responseStreamConnectionFailed",
    "responseStreamDisconnected",
    "responseTooManyFailedAttempts",
    "internalServerError",
    "badRequest",
    "sandboxError",
    "invalid_request",
    "server_error",
    "max_tokens",
    "max_turn_requests",
    "refusal",
    super::MODEL_SWITCH_REFUSED,
];

/// Lower-cased needles for a failure without a deciding code (Claude's
/// result frames carry only a subtype; ACP agents a bare JSON-RPC number).
const USAGE_TEXT: &[&str] = &[
    "usage limit",
    "rate limit",
    "rate_limit",
    "quota",
    "limit reached",
    "hit your limit",
    "too many requests",
    "resource_exhausted",
    "resource exhausted",
    "credit balance",
];
const AUTH_TEXT: &[&str] = &[
    "/login",
    "not logged in",
    "log in",
    "login required",
    "sign in",
    "invalid api key",
    "api key",
    "unauthorized",
    "unauthenticated",
    "authentication",
    "oauth token",
];

/// The coarse class of a turn's failure: `usage_limit`, `auth` or `provider`.
pub fn error_class(f: &ProviderFailure) -> &'static str {
    let code = f.code.as_deref().map(|c| c.trim().trim_matches('"'));
    match code {
        Some(c) if USAGE_CODES.contains(&c) => return ERROR_CLASS_USAGE_LIMIT,
        Some(c) if AUTH_CODES.contains(&c) => return ERROR_CLASS_AUTH,
        Some(c) if PROVIDER_CODES.contains(&c) => return ERROR_CLASS_PROVIDER,
        _ => {}
    }
    let text = f.message.to_lowercase();
    if USAGE_TEXT.iter().any(|n| text.contains(n)) {
        ERROR_CLASS_USAGE_LIMIT
    } else if AUTH_TEXT.iter().any(|n| text.contains(n)) {
        ERROR_CLASS_AUTH
    } else {
        ERROR_CLASS_PROVIDER
    }
}

/// Which kind of limit a `usage_limit` failure is ([`limit_kind`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    /// A per-minute 429: retry with backoff.
    Rate,
    /// The plan's usage limit; `resets_at` when the failure itself says.
    Plan {
        resets_at: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// No credit left: nothing resets it.
    Billing,
}

const RATE_CODES: &[&str] = &["rate_limit", "rate_limit_error"];
const RATE_TEXT: &[&str] = &["rate limit", "rate_limit", "too many requests", "429"];

/// Tell a per-minute rate limit, a plan limit and a billing failure apart;
/// `None` when [`error_class`] is not `usage_limit`. The row's
/// `last_error_class` keeps the one coarse class.
pub fn limit_kind(f: &ProviderFailure) -> Option<LimitKind> {
    if error_class(f) != ERROR_CLASS_USAGE_LIMIT {
        return None;
    }
    let code = f.code.as_deref().map(|c| c.trim().trim_matches('"'));
    let text = f.message.to_lowercase();
    if code == Some("billing_error") || text.contains("credit balance") {
        return Some(LimitKind::Billing);
    }
    let planned = code.is_some_and(|c| !RATE_CODES.contains(&c) && USAGE_CODES.contains(&c));
    if !planned
        && (code.is_some_and(|c| RATE_CODES.contains(&c))
            || RATE_TEXT.iter().any(|n| text.contains(n)))
    {
        return Some(LimitKind::Rate);
    }
    let resets_at = reset_in_text(&f.message, chrono::Local::now());
    Some(LimitKind::Plan { resets_at })
}

/// When a limit resets, from the CLI's own words (the shapes the Threads
/// view reads, ui/src/modules/threads/runtime/usageLimit.ts): Claude's
/// "…usage limit reached|<unix seconds>" and "…limit reached ∙ resets 3pm"
/// / "resets at 14:00", Codex's "try again in 2 hours 5 minutes". A clock
/// time is local (the CLI's own); one already past today means tomorrow.
pub fn reset_in_text(
    text: &str,
    now: chrono::DateTime<chrono::Local>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Some(t) = text
        .rsplit_once('|')
        .and_then(|(_, t)| t.trim().parse().ok())
    {
        return chrono::DateTime::from_timestamp(t, 0);
    }
    let text = text.to_lowercase();
    let after = |lead: &str| text.find(lead).map(|i| &text[i + lead.len()..]);
    if let Some(rest) = after("try again in ").or_else(|| after("retry in ")) {
        let secs = relative_secs(rest)?;
        return Some(now.to_utc() + chrono::Duration::milliseconds((secs * 1000.0) as i64));
    }
    let rest = after("resets at ").or_else(|| after("resets "))?;
    let (h, rest) = leading_number(rest)?;
    let (m, rest) = match rest.strip_prefix(':') {
        Some(r) => leading_number(r).map_or((None, r), |(m, r)| (Some(m), r)),
        None => (None, rest),
    };
    let rest = rest.trim_start();
    let pm = rest.starts_with("pm");
    if !pm && !rest.starts_with("am") && m.is_none() {
        return None; // "resets 3" is not a time
    }
    let h = match (h as u32, pm) {
        (12, false) if rest.starts_with("am") => 0,
        (h, true) if h < 12 => h + 12,
        (h, _) => h,
    };
    let at = now
        .date_naive()
        .and_hms_opt(h, m.unwrap_or(0.0) as u32, 0)?
        .and_local_timezone(chrono::Local)
        .earliest()?;
    let at = if at <= now {
        at + chrono::Duration::days(1)
    } else {
        at
    };
    Some(at.to_utc())
}

/// "2 hours 5 minutes", "41.2s" → seconds.
fn relative_secs(mut rest: &str) -> Option<f64> {
    let mut total = 0.0;
    while let Some((n, r)) = leading_number(rest) {
        let r = r.trim_start();
        let unit: String = r.chars().take_while(char::is_ascii_alphabetic).collect();
        let scale = match unit.as_str() {
            "d" | "day" | "days" => 86_400.0,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3_600.0,
            "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
            "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
            _ => break,
        };
        total += n * scale;
        rest = r[unit.len()..].trim_start_matches([' ', ',']);
        rest = rest.strip_prefix("and ").unwrap_or(rest);
    }
    (total > 0.0).then_some(total)
}

/// The number at the start of `s` and what follows it.
fn leading_number(s: &str) -> Option<(f64, &str)> {
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    Some((s[..end].parse().ok()?, &s[end..]))
}

impl RuntimeSession {
    /// Set [`Self::last_turn_status`] / [`Self::last_error_class`] from the
    /// thread's latest turn (by ordinal) and that turn's failure, if any. A
    /// turn still running clears both: the row shows the attempt under way.
    pub fn set_outcome(
        &mut self,
        latest: Option<&ProviderTurn>,
        failure: Option<&ProviderFailure>,
    ) {
        self.last_turn_status = latest.and_then(|t| TurnOutcome::of(t.status));
        self.last_error_class = match self.last_turn_status {
            Some(TurnOutcome::Failed) => failure.map(|f| error_class(f).to_string()),
            _ => None,
        };
        // When that turn ended (`meta.last_turn_ended_at`, derived, never
        // stored): a client's "finished since you last looked" compares it with
        // `meta.thread_seen_at` — `last_active_at` also moves on a pin.
        let ended = self
            .last_turn_status
            .and(latest)
            .and_then(|t| t.completed_at);
        self.set_derived_time(TURN_ENDED_AT_META, ended);
        // When the latest turn started (`meta.last_turn_started_at`, derived the
        // same way): a row that was already working when the page loaded shows
        // "Working 4m" without reading the thread.
        self.set_derived_time(TURN_STARTED_AT_META, latest.and_then(|t| t.started_at));
    }

    /// Set (or drop) a derived time key in `meta`.
    fn set_derived_time(&mut self, key: &str, at: Option<chrono::DateTime<chrono::Utc>>) {
        let at = at.map(|t| serde_json::Value::String(t.to_rfc3339()));
        match (
            self.meta
                .as_mut()
                .and_then(serde_json::Value::as_object_mut),
            at,
        ) {
            (Some(m), Some(at)) => {
                m.insert(key.into(), at);
            }
            (Some(m), None) => {
                m.remove(key);
            }
            (None, Some(at)) => self.meta = Some(serde_json::json!({ key: at })),
            (None, None) => {}
        }
    }
}

/// Derived `RuntimeSession::meta` key: when the latest turn ended (RFC 3339).
pub const TURN_ENDED_AT_META: &str = "last_turn_ended_at";
/// Derived `RuntimeSession::meta` key: when the latest turn started (RFC 3339),
/// kept while it runs (unlike [`TURN_ENDED_AT_META`]).
pub const TURN_STARTED_AT_META: &str = "last_turn_started_at";

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(code: Option<&str>, message: &str) -> ProviderFailure {
        ProviderFailure {
            class: "provider_error".into(),
            message: message.into(),
            code: code.map(str::to_string),
            retryable: None,
        }
    }

    #[test]
    fn the_latest_ended_turn_dates_the_row_and_a_running_one_clears_it() {
        let mut s: RuntimeSession = serde_json::from_value(serde_json::json!({
            "id": "s", "workspace_id": "w", "provider": "claude", "title": "t", "cwd": "/",
            "status": "idle", "access_mode": "full-access", "created_by": "u",
            "created_at": "2026-11-23T10:00:00Z", "last_active_at": "2026-11-23T10:00:00Z",
            "last_sequence": 1, "capabilities": {}, "meta": {"thread_pinned": true}
        }))
        .unwrap();
        let mut turn: ProviderTurn = serde_json::from_value(serde_json::json!({
            "id": "t1", "ordinal": 1, "status": "completed",
            "started_at": "2026-11-23T10:00:00Z", "completed_at": "2026-11-23T10:05:00Z"
        }))
        .unwrap();
        s.set_outcome(Some(&turn), None);
        assert_eq!(
            s.meta.as_ref().unwrap()[TURN_ENDED_AT_META],
            "2026-11-23T10:05:00+00:00"
        );
        assert_eq!(s.meta.as_ref().unwrap()["thread_pinned"], true);
        turn.status = ProviderTurnStatus::Running;
        turn.completed_at = None;
        s.set_outcome(Some(&turn), None);
        assert!(s.meta.as_ref().unwrap().get(TURN_ENDED_AT_META).is_none());
        // A running turn keeps its start (a row's "Working 4m" after a reload).
        assert_eq!(
            s.meta.as_ref().unwrap()[TURN_STARTED_AT_META],
            "2026-11-23T10:00:00+00:00"
        );
        s.set_outcome(None, None);
        assert!(s.meta.as_ref().unwrap().get(TURN_STARTED_AT_META).is_none());
    }

    #[test]
    fn a_structured_code_decides_the_class_before_the_text() {
        assert_eq!(
            error_class(&failure(Some("usageLimitExceeded"), "boom")),
            "usage_limit"
        );
        assert_eq!(error_class(&failure(Some("unauthorized"), "x")), "auth");
        assert_eq!(error_class(&failure(Some("-32000"), "x")), "auth");
        // A listed provider code wins over words that look like a limit.
        assert_eq!(
            error_class(&failure(Some("contextWindowExceeded"), "rate limit")),
            "provider"
        );
    }

    #[test]
    fn limits_are_told_apart() {
        use chrono::TimeZone;
        let kind = |code: Option<&str>, text: &str| limit_kind(&failure(code, text));
        assert_eq!(kind(Some("rate_limit_error"), "x"), Some(LimitKind::Rate));
        assert_eq!(
            kind(None, "API Error: 429 Too Many Requests"),
            Some(LimitKind::Rate)
        );
        assert_eq!(
            kind(None, "Rate limit exceeded, retry in 20s"),
            Some(LimitKind::Rate)
        );
        assert_eq!(
            kind(Some("success"), "Claude AI usage limit reached|1700000000"),
            Some(LimitKind::Plan {
                resets_at: Some(chrono::Utc.timestamp_opt(1_700_000_000, 0).unwrap())
            })
        );
        assert!(matches!(
            kind(
                Some("usageLimitExceeded"),
                "You've hit your usage limit · resets 3pm"
            ),
            Some(LimitKind::Plan { resets_at: Some(_) })
        ));
        assert_eq!(
            kind(Some("usageLimitExceeded"), "You've hit your usage limit"),
            Some(LimitKind::Plan { resets_at: None })
        );
        assert_eq!(
            kind(Some("insufficient_quota"), "rate limit"),
            Some(LimitKind::Plan { resets_at: None }),
            "a plan code decides over words"
        );
        assert_eq!(
            kind(None, "Credit balance is too low"),
            Some(LimitKind::Billing)
        );
        assert_eq!(kind(Some("billing_error"), "x"), Some(LimitKind::Billing));
        assert_eq!(kind(None, "API Error: 500 Internal server error"), None);
    }

    /// The CLIs' own reset words (ui/unit/runtimeUsageLimit.test.ts's cases).
    #[test]
    fn the_reset_is_read_from_the_providers_words() {
        use chrono::TimeZone;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 11, 23, 12, 0, 0)
            .unwrap();
        let local = |d, h, m| {
            let t = chrono::Local
                .with_ymd_and_hms(2026, 11, d, h, m, 0)
                .unwrap();
            Some(t.to_utc())
        };
        let at = |text: &str| reset_in_text(text, now);
        assert_eq!(
            at("Claude AI usage limit reached|1763906400"),
            chrono::DateTime::from_timestamp(1_763_906_400, 0)
        );
        assert_eq!(at("5-hour limit reached ∙ resets 3pm"), local(23, 15, 0));
        assert_eq!(
            at("You've hit your usage limit · resets 3pm (Europe/London)"),
            local(23, 15, 0)
        );
        assert_eq!(
            at("limit reached, resets at 09:30"),
            local(24, 9, 30),
            "past today: tomorrow"
        );
        assert_eq!(at("resets 12am"), local(24, 0, 0));
        let later = |s: i64| Some(now.to_utc() + chrono::Duration::seconds(s));
        assert_eq!(
            at("You've hit your usage limit. Upgrade to Pro or try again in 2 hours 5 minutes."),
            later(2 * 3600 + 5 * 60)
        );
        assert_eq!(
            at("Please retry in 41.2s."),
            Some(now.to_utc() + chrono::Duration::milliseconds(41_200))
        );
        for none in [
            "usage limit reached",
            "resets 3",
            "You've hit your usage limit. Try again later.",
        ] {
            assert_eq!(at(none), None, "{none}");
        }
    }

    #[test]
    fn without_a_deciding_code_the_message_is_read() {
        let claude_limit = failure(Some("success"), "Claude AI usage limit reached|1700000000");
        assert_eq!(error_class(&claude_limit), "usage_limit");
        let signed_out = failure(
            Some("error_during_execution"),
            "Not logged in · Please run /login",
        );
        assert_eq!(error_class(&signed_out), "auth");
        assert_eq!(
            error_class(&failure(Some("-32603"), "Quota exceeded for model")),
            "usage_limit"
        );
        assert_eq!(
            error_class(&failure(None, "unsupported control request: initialize")),
            "provider"
        );
    }

    #[test]
    fn a_cancelled_turn_reads_as_interrupted_and_a_running_one_as_none() {
        assert_eq!(
            TurnOutcome::of(ProviderTurnStatus::Cancelled),
            Some(TurnOutcome::Interrupted)
        );
        assert_eq!(TurnOutcome::of(ProviderTurnStatus::Running), None);
        assert_eq!(
            serde_json::to_value(TurnOutcome::Interrupted).unwrap(),
            "interrupted"
        );
    }
}
