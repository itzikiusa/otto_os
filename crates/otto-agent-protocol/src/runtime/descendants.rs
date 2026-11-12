//! Jobs a Claude CLI moved out of its process group are stopped with it.
//!
//! The runtime stops an agent by signalling its process group, but Claude
//! Code starts some jobs (background commands) in a group of their own, and
//! once the CLI exits they are re-parented and nothing finds them again. So
//! while a Claude session works (a turn or background work runs) its process
//! tree is recorded every [`POLL`] from `ps -A -o pid=,ppid=,pgid=,lstart=`
//! (also the first tick after any activity, so a short turn's job is seen);
//! when that process is gone (closed, exited, replaced) every recorded job
//! still running — the same pid AND start time, so a reused pid is never hit
//! — is SIGKILLed, a job leading its own group with that group. Ported from
//! T3 Code #34505 (`ClaudeAdapterV2.ts` `trackDescendants`), which polls
//! always; Otto polls only while the session is busy, so an idle thread
//! costs nothing.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use super::Live;
use crate::process::signal_group;

/// How often a busy session's process tree is recorded.
const POLL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    pid: u32,
    ppid: u32,
    pgid: u32,
    /// `lstart`: with the pid, tells a process from a reused pid.
    started: String,
}

impl Row {
    fn identity(&self) -> (u32, String) {
        (self.pid, self.started.clone())
    }
}

/// `ps -A -o pid=,ppid=,pgid=,lstart=` → rows (unparseable lines skipped).
pub(super) fn parse(table: &str) -> Vec<Row> {
    table
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let pid = f.next()?.parse().ok()?;
            let ppid = f.next()?.parse().ok()?;
            let pgid = f.next()?.parse().ok()?;
            let started = f.collect::<Vec<_>>().join(" ");
            (!started.is_empty()).then_some(Row {
                pid,
                ppid,
                pgid,
                started,
            })
        })
        .collect()
}

/// Every process below `roots` (children, their children, …).
pub(super) fn descendants(rows: &[Row], roots: &[u32]) -> Vec<Row> {
    let mut children: HashMap<u32, Vec<&Row>> = HashMap::new();
    for r in rows {
        children.entry(r.ppid).or_default().push(r);
    }
    let mut seen: HashSet<u32> = roots.iter().copied().collect();
    let mut pending: Vec<u32> = roots.to_vec();
    let mut found = Vec::new();
    while let Some(parent) = pending.pop() {
        for child in children.get(&parent).into_iter().flatten() {
            if seen.insert(child.pid) {
                found.push((*child).clone());
                pending.push(child.pid);
            }
        }
    }
    found
}

async fn read_table() -> Vec<Row> {
    let mut ps = tokio::process::Command::new("ps");
    ps.args(["-A", "-o", "pid=,ppid=,pgid=,lstart="])
        .kill_on_drop(true);
    match tokio::time::timeout(Duration::from_secs(5), ps.output()).await {
        Ok(Ok(o)) if o.status.success() => parse(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

/// The CLI's recorded tree.
#[derive(Default)]
pub(super) struct Tracker {
    /// The CLI's own identity, once seen (its pid alone may be reused).
    cli: Option<(u32, String)>,
    tracked: HashMap<(u32, String), Row>,
}

impl Tracker {
    /// Record what runs below the CLI (while it is still the same process)
    /// and below the jobs already recorded. Returns who is running now.
    pub(super) fn refresh(&mut self, rows: &[Row], cli_pid: u32) -> HashSet<(u32, String)> {
        let running: HashSet<(u32, String)> = rows.iter().map(Row::identity).collect();
        if self.cli.is_none() {
            self.cli = rows.iter().find(|r| r.pid == cli_pid).map(Row::identity);
        }
        let mut roots: Vec<u32> = self
            .tracked
            .values()
            .filter(|r| running.contains(&r.identity()))
            .map(|r| r.pid)
            .collect();
        if self.cli.as_ref().is_some_and(|c| running.contains(c)) {
            roots.push(cli_pid);
        }
        for r in descendants(rows, &roots) {
            self.tracked.insert(r.identity(), r);
        }
        running
    }

    /// The recorded jobs still running: the ones to stop.
    pub(super) fn survivors(&self, running: &HashSet<(u32, String)>) -> Vec<Row> {
        let mut v: Vec<Row> = self
            .tracked
            .values()
            .filter(|r| running.contains(&r.identity()))
            .cloned()
            .collect();
        v.sort_by_key(|r| r.pid);
        v
    }
}

fn kill(row: &Row) {
    if row.pgid == row.pid {
        signal_group(row.pid, libc::SIGKILL);
    } else if let Ok(pid) = libc::pid_t::try_from(row.pid) {
        if pid > 1 {
            // SAFETY: plain syscall; the pid's start time was just checked.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

/// Track the Claude CLI `pid` of process life `epoch` until that life ends,
/// then stop the jobs it left behind.
pub(super) fn watch(live: Arc<Live>, epoch: u64, pid: u32) {
    tokio::spawn(async move {
        let mut tracker = Tracker::default();
        let mut last_seq = None;
        loop {
            tokio::time::sleep(POLL).await;
            let (alive, busy, seq) = {
                let st = live.lock();
                let alive = st.proc.as_ref().is_some_and(|p| p.epoch == epoch);
                let busy = st.turn.is_some() || !st.session.background_tasks.is_empty();
                (alive, busy, st.next_seq)
            };
            if !alive {
                break;
            }
            // Busy, or anything happened since the last tick (a turn that
            // started and ended between two ticks may have left a job).
            if busy || last_seq != Some(seq) {
                tracker.refresh(&read_table().await, pid);
            }
            last_seq = Some(seq);
        }
        if tracker.tracked.is_empty() {
            return; // nothing was ever recorded below it
        }
        let rows = read_table().await;
        let running = tracker.refresh(&rows, pid);
        for row in tracker.survivors(&running) {
            kill(&row);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
      1     0     1 Thu Nov 12 09:00:00 2026
    100     1   100 Thu Nov 12 10:00:00 2026
    101   100   100 Thu Nov 12 10:00:01 2026
    102   101   102 Thu Nov 12 10:00:02 2026
    103   102   102 Thu Nov 12 10:00:03 2026
    200     1   200 Thu Nov 12 10:00:04 2026
garbage line
";

    #[test]
    fn the_cli_tree_is_found_through_jobs_in_their_own_group() {
        let rows = parse(TABLE);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[1].started, "Thu Nov 12 10:00:00 2026");
        let mut pids: Vec<u32> = descendants(&rows, &[100]).iter().map(|r| r.pid).collect();
        pids.sort_unstable();
        assert_eq!(pids, [101, 102, 103]);
    }

    #[test]
    fn a_job_reparented_after_the_cli_exits_is_still_stopped_but_a_reused_pid_is_not() {
        let mut t = Tracker::default();
        t.refresh(&parse(TABLE), 100);
        // The CLI exited: its job (own group) was re-parented to launchd,
        // and the CLI's pid now belongs to someone else.
        let after = parse(
            "\
    100     1   100 Thu Nov 12 11:00:00 2026
    102     1   102 Thu Nov 12 10:00:02 2026
    103   102   102 Thu Nov 12 10:00:03 2026
    104   102   102 Thu Nov 12 10:30:00 2026
    101     1   101 Thu Nov 12 11:00:01 2026
",
        );
        let running = t.refresh(&after, 100);
        let pids: Vec<u32> = t.survivors(&running).iter().map(|r| r.pid).collect();
        // 101 is a new process on a reused pid; the new 100 is not the CLI.
        assert_eq!(pids, [102, 103, 104]);
    }
}
