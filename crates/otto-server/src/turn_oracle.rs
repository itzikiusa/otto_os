//! Turn oracle — decides when an agent-backed workflow step is REALLY done.
//!
//! A claude `end_turn` is not "finished": the parent ends a turn every time a
//! sub-agent reports back (`<task-notification>`), and it ends one right after
//! launching sub-agents ("waiting on the four sweep agents"). Completion is
//! therefore: turn ended natively + no launched/resumed task pending + the
//! handoff file written, each hold bounded. Pure functions over the transcript
//! text plus a small clock struct; the only I/O is `stat`/read of files the
//! caller hands in. Seeded surface — bodies land with WP1 of the workflows
//! batch; `agent_run` (review engine) consumes `scan_claude`/`progress_stamp`.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// What kind of harness task a pending id refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    /// `Agent` tool launch (`toolUseResult.status == "async_launched"`) or a
    /// `SendMessage` resume (`toolUseResult.resumedAgentId`).
    Agent,
    /// `Bash` with `run_in_background` (`toolUseResult.backgroundTaskId`).
    Bash,
}

/// A task the parent launched/resumed that has not reported back yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTask {
    pub id: String,
    pub kind: TaskKind,
    pub description: String,
    /// 0-based transcript line the launch was seen on.
    pub since_line: usize,
}

/// One pass over a claude session JSONL.
#[derive(Debug, Clone, Default)]
pub struct ClaudeScan {
    /// `completed_turn_count` semantics, skipping `isSidechain:true` lines.
    pub completed_turns: usize,
    pub last_turn_text: Option<String>,
    /// `timestamp` of the last MESSAGE-bearing line (idle-confirm clock).
    pub last_message_at: Option<SystemTime>,
    /// The last message line is an assistant `end_turn` and nothing after it
    /// is a message line or carries a `<task-notification>`.
    pub tail_is_assistant_end_turn: bool,
    /// The only post-end_turn lines are `queue-operation` lines.
    pub tail_evidence_is_queue_only: bool,
    /// Launched/resumed ids with no notification yet.
    pub pending: Vec<PendingTask>,
    /// id → last `<status>` seen (`completed|failed|killed|stopped`).
    pub notified: BTreeMap<String, String>,
    /// `transcript_api_error` verbatim.
    pub api_error: Option<String>,
}

/// Live phase of a step's agent turn (drives the log lines + `activity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Booting,
    PromptAccepted,
    Working,
    Subagents { running: usize, done: usize },
    IdleConfirming { left: Duration },
    HandoffWrittenWaiting { pending: usize },
    HandoffMissingGrace { left: Duration },
    BashLinger { pending: usize, left: Duration },
}

/// Walk a claude session JSONL once (seed: no signal).
pub fn scan_claude(_jsonl: &str) -> ClaudeScan {
    ClaudeScan::default()
}

/// Newest mtime over the main transcript, `subagents/*.jsonl` and
/// `tasks/*.output` — the stall clock. `None` when nothing exists yet
/// (never "no progress"). Seed: `None`.
pub fn progress_stamp(
    _main: &Path,
    _subagent_dir: Option<&Path>,
    _tasks_dir: Option<&Path>,
) -> Option<SystemTime> {
    None
}
