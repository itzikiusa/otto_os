//! Process-wide "the daemon is stopping" signal.
//!
//! launchd SIGKILLs the daemon `ExitTimeOut` seconds after SIGTERM, and the
//! HTTP server's graceful drain waits for every in-flight request — including
//! long-polls that legitimately block for 25–30 s (`/sessions/{id}/wait`, an
//! MCP call waiting on a human approval). Those handlers race their wait
//! against [`cancelled`] so a shutdown answers them at once (with their normal
//! "not reached yet" result; callers already loop) instead of holding the
//! drain open. ottod calls [`begin`] when the shutdown signal arrives.

use std::sync::LazyLock;

use crate::cancel_signal::CancelSignal;

static SHUTDOWN: LazyLock<CancelSignal> = LazyLock::new(CancelSignal::new);

/// Mark the process as shutting down; wakes every [`cancelled`] waiter.
pub fn begin() {
    SHUTDOWN.cancel();
}

/// True once [`begin`] has been called.
pub fn is_shutting_down() -> bool {
    SHUTDOWN.is_cancelled()
}

/// Resolves when the daemon starts shutting down.
pub async fn cancelled() {
    SHUTDOWN.cancelled().await;
}
