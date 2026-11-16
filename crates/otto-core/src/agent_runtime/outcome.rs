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
    }
}

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
