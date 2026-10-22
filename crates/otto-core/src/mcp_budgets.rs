//! Wall-clock budgets of Otto's MCP tool calls, shared by the stdio bridge
//! (`ottod mcp-tools`, which enforces them per upstream request) and the
//! session spawner (`otto-sessions::mcp`, which tells a client such as codex
//! how long to wait for one tool call). One home so the client-side cap can
//! never fall below a bridge budget again (integrations-4-03: codex's 60 s
//! default cut `browser_summarize`, governed runs and log fetches off while
//! the daemon kept working, and a mutating call invited a duplicate retry).

use std::time::Duration;

/// Default budget of one upstream daemon request from the bridge. The
/// `otto_db_query` read uses it, so it stays at or above 20 s: the daemon's
/// `/db/mcp-query` stops the statement itself at 15 s (PC-8) and must answer
/// before the bridge gives up.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// A text (pod log) fetch: the non-follow logs route runs kubectl with a 60 s
/// budget of its own.
pub const TEXT_CALL_TIMEOUT: Duration = Duration::from_secs(65);

/// `browser_summarize`: the route runs a whole agent turn (spawn the CLI, wait
/// for the TUI, run the turn); 15 min covers a cold spawn and a slow model.
pub const SUMMARIZE_CALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// How long a governed call waits for a human decision before returning
/// `pending_approval` — below [`CALL_TIMEOUT`] so the daemon answers first.
pub const GOVERNED_WAIT_SECS: u64 = 15;

/// What the governed proxy allows the tool's own run after the approval wait:
/// the longest per-tool self-call budget (`api_run_automation`, 180 s) plus a
/// margin.
pub const GOVERNED_RUN_BUDGET: Duration = Duration::from_secs(190);

/// The per-tool timeout a client is told to allow for Otto's own MCP servers
/// (`otto`, `otto-browser`): the longest bridge budget plus a minute, so the
/// bridge (and the daemon behind it) always answers before the client gives up.
pub const CLIENT_TOOL_TIMEOUT_SECS: u64 = SUMMARIZE_CALL_TIMEOUT.as_secs() + 60;

/// The per-tool timeout a client is told to allow for a user-configured MCP
/// server that states none: generous (a downstream Jira/GitHub server may run
/// a slow search), but bounded so a hung server still surfaces.
pub const USER_SERVER_TOOL_TIMEOUT_SECS: u64 = 600;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_cap_covers_every_bridge_budget() {
        let client = Duration::from_secs(CLIENT_TOOL_TIMEOUT_SECS);
        for budget in [
            CALL_TIMEOUT,
            TEXT_CALL_TIMEOUT,
            SUMMARIZE_CALL_TIMEOUT,
            Duration::from_secs(GOVERNED_WAIT_SECS) + GOVERNED_RUN_BUDGET,
        ] {
            assert!(client > budget, "{client:?} <= {budget:?}");
        }
        // PC-8: the db query budget stays at or above 20 s.
        assert!(CALL_TIMEOUT >= Duration::from_secs(20));
        assert!(Duration::from_secs(GOVERNED_WAIT_SECS) < CALL_TIMEOUT);
    }
}
