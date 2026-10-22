//! Per-route upstream budgets for the stdio bridge (integrations-3-01) and the
//! pure [`ReadCall`] descriptor `read_route` builds: a route that blocks by
//! design (the `otto_wait_session` long-poll, the `browser_summarize` agent
//! turn) states its own budget instead of inheriting [`CALL_TIMEOUT`].

use std::time::Duration;

use serde_json::Value;

use super::CALL_TIMEOUT;

/// Longest wait `otto_wait_session` forwards (the daemon route's own cap,
/// `session_input.rs`: `timeout_secs.unwrap_or(20).min(25)`).
pub(super) const WAIT_SESSION_MAX_SECS: u64 = 25;
/// Default `timeout_secs` of `otto_wait_session` (the daemon's default).
pub(super) const WAIT_SESSION_DEFAULT_SECS: u64 = 20;
/// Headroom over the daemon-side wait so the daemon's own `{reached:false}`
/// answer always lands before this bridge gives up (at 20 s it used to race
/// the bridge's 20 s timer and lose — integrations-3-01).
pub(super) const WAIT_SESSION_MARGIN: Duration = Duration::from_secs(5);
/// Budget for `browser_summarize`: the route runs a whole agent turn (spawn
/// the CLI, wait for the TUI, run the turn), and dropping the request kills
/// that agent. The route's own idle backstop is the 10 h `STUCK_IDLE`, which
/// is no useful bound for one tool call; a page summary is a single short
/// turn, so 15 min covers a cold spawn and a slow model with wide margin.
pub(super) const SUMMARIZE_CALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The bridge budget for one `otto_wait_session` call: the wait the daemon
/// will actually do (`timeout_secs` clamped to 1..=25, default 20) plus
/// [`WAIT_SESSION_MARGIN`].
pub(super) fn wait_session_budget(args: &Value) -> Duration {
    let secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64().or_else(|| v.as_str()?.trim().parse().ok()))
        .unwrap_or(WAIT_SESSION_DEFAULT_SECS)
        .clamp(1, WAIT_SESSION_MAX_SECS);
    Duration::from_secs(secs) + WAIT_SESSION_MARGIN
}

/// A resolved upstream read call: GET (or a read-only viewer POST), the `/api/v1`-
/// relative path, and an optional JSON body. Built purely from `(name, args, ws)`
/// by `read_route` so each feature read's endpoint binding is unit-tested.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ReadCall {
    pub(super) post: bool,
    pub(super) path: String,
    pub(super) body: Option<Value>,
    /// Wall-clock budget for the upstream call ([`CALL_TIMEOUT`] unless the
    /// route blocks by design, like the `otto_wait_session` long-poll).
    pub(super) budget: Duration,
}

impl ReadCall {
    pub(super) fn get(path: String) -> Self {
        Self {
            post: false,
            path,
            body: None,
            budget: CALL_TIMEOUT,
        }
    }
    pub(super) fn post(path: String, body: Value) -> Self {
        Self {
            post: true,
            path,
            body: Some(body),
            budget: CALL_TIMEOUT,
        }
    }
    pub(super) fn within(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }
}
