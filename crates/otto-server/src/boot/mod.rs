//! Daemon composition root: everything `ottod` does between "the
//! single-instance lock is held" and "the router is built".
//!
//! The sequence (driven by `ottod::run`; each step is awaited before the
//! next, and the ORDER is load-bearing):
//!
//! 1. [`open_state`] — offline compaction, open + migrate the state DB,
//!    interrupted-change recovery, boot maintenance.
//! 2. [`build_ctx`] — construct every module (providers, ClickHouse usage
//!    engine, session manager, …) and assemble the [`ServerCtx`] through
//!    [`ServerCtx::from_parts`], the literal the test fixture shares.
//! 3. [`recover_before_serve`] → [`spawn_post_listen_work`] →
//!    [`recover_goal_loops`] — settle what the previous daemon life left
//!    behind (sessions, reviews, evals, API runs, workflows, goal loops).
//! 4. [`spawn_background`] — sweeps, retention and every scheduler; the
//!    reaps that must precede serving are awaited inside it.
//!
//! Nothing here binds a socket, owns the process lifecycle or touches the
//! binary's own files (logs, the running marker, the usage tailer) — that
//! stays in `ottod`.

mod build;
mod ctx;
mod open;
mod recovery;
mod tasks;

use std::path::PathBuf;

pub use build::{build_ctx, BuiltCtx};
pub use ctx::CtxParts;
pub use open::open_state;
pub use recovery::{recover_before_serve, recover_goal_loops, spawn_post_listen_work};
pub use tasks::{spawn_background, Background};

/// What the composition root needs from the binary.
#[derive(Debug, Clone)]
pub struct BootConfig {
    /// The daemon data directory (library, worktrees, ClickHouse, …).
    pub data_dir: PathBuf,
    /// The SQLite state DB (`<data_dir>/otto.db`).
    pub db_path: PathBuf,
    /// Loopback port (agent hooks and plugin sidecars call back on it).
    pub port: u16,
    /// Reported by `/meta`.
    pub version: String,
    /// PTY-holder setup for session persistence; `None` disables it.
    pub pty_holders: Option<otto_pty::HolderConfig>,
}

/// Wall time per boot phase (perf2/03 N4/N7). `finish` logs ONE line —
/// `boot: ready in N ms (db_compact=… db_open=… …)` — that the perf budget
/// script (scripts/perf/daemon-budget.mjs) parses for its boot_ms and
/// per-phase budgets.
pub struct BootPhases {
    started: std::time::Instant,
    last: std::time::Instant,
    phases: Vec<(&'static str, u128)>,
}

impl BootPhases {
    pub fn start() -> Self {
        let now = std::time::Instant::now();
        Self {
            started: now,
            last: now,
            phases: Vec::new(),
        }
    }

    /// Close the phase that ended now.
    pub fn mark(&mut self, phase: &'static str) {
        let now = std::time::Instant::now();
        self.phases.push((phase, (now - self.last).as_millis()));
        self.last = now;
    }

    fn line(&self) -> String {
        let parts: Vec<String> = self
            .phases
            .iter()
            .map(|(p, ms)| format!("{p}={ms}"))
            .collect();
        format!(
            "boot: ready in {} ms ({})",
            self.started.elapsed().as_millis(),
            parts.join(" ")
        )
    }

    /// Close the trailing `recovery` phase and log the `boot: ready` line.
    pub fn finish(&mut self) {
        self.mark("recovery");
        tracing::info!("{}", self.line());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_line_lists_every_phase_in_order() {
        let mut b = BootPhases::start();
        b.mark("db_open");
        b.mark("modules");
        b.mark("recovery");
        let line = b.line();
        assert!(line.starts_with("boot: ready in "), "{line}");
        let open = line.find("db_open=").expect("db_open phase");
        let modules = line.find("modules=").expect("modules phase");
        let recovery = line.find("recovery=").expect("recovery phase");
        assert!(open < modules && modules < recovery, "{line}");
    }
}
