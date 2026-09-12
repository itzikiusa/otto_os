//! Turn oracle — decides when an agent-backed workflow step is REALLY done.
//!
//! A claude `end_turn` is not "finished": the parent ends a turn every time a
//! sub-agent reports back (`<task-notification>`), and it ends one right after
//! launching sub-agents ("waiting on the four sweep agents"). Completion is
//! therefore: turn ended natively + no launched/resumed task pending + the
//! handoff file written, each hold bounded. Pure functions over the transcript
//! text plus a small clock struct; the only I/O is `stat`/read of files the
//! caller hands in. `agent_session` drives it per poll tick for workflow steps;
//! `agent_run` (review engine) consumes `scan_claude`/`progress_stamp`.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

/// How long the transcript must stay quiet after a native end-turn + handoff
/// before the step is accepted (rule 3). Keyed on the last MESSAGE line's
/// timestamp (and any sub-agent jsonl moving), never on the file's mtime.
pub const IDLE_CONFIRM: Duration = Duration::from_secs(20);
/// The turn ended but no handoff file exists (rule 4): nudge once, then accept
/// the final reply rather than hanging the run.
pub const HANDOFF_MISSING_GRACE: Duration = Duration::from_secs(90);
/// The handoff exists but the turn has not ended (rule 5): wait this long for
/// the stragglers, then move on.
pub const HANDOFF_LINGER_CAP: Duration = Duration::from_secs(15 * 60);
/// The parent is idle with only background `Bash` ids left (rule 5b) — a dev
/// server / watcher it forgot to stop would otherwise run to the 10h cap.
pub const BASH_LINGER_CAP: Duration = Duration::from_secs(15 * 60);
/// A `<task-notification>` that only ever appeared as a `queue-operation` line
/// (never delivered — a `remove`d one never is) stops blocking the tail after
/// this long without a new message line.
pub const QUEUE_ONLY_BOUND: Duration = Duration::from_secs(60);
/// How long into [`HANDOFF_MISSING_GRACE`] the "write your handoff now" nudge
/// is submitted (once).
pub const NUDGE_AFTER: Duration = Duration::from_secs(10);

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
    /// `timestamp` of the line that carried each id's notification — what gives
    /// the Agents tab a sub-agent's duration. Ids whose notification line had no
    /// parseable timestamp are absent.
    pub notified_at: BTreeMap<String, SystemTime>,
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

/// Display status of one sub-agent / background task. Taken ONLY from the
/// parent transcript — a child's own jsonl routinely ends `stop_reason:null`
/// (or does not exist), so it is never a completion signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubStatus {
    Running,
    Done,
    Failed,
}

/// One sub-agent / background task, for the run view's nested rows.
#[derive(Debug, Clone)]
pub struct SubagentInfo {
    pub id: String,
    pub description: String,
    pub status: SubStatus,
    pub started_at: Option<SystemTime>,
    pub finished_at: Option<SystemTime>,
}

/// One pass over a codex rollout, limited to lines past the submit baseline.
#[derive(Debug, Clone, Default)]
pub struct CodexScan {
    /// `turn_id` of the LATEST `task_started`; `None` after its `turn_aborted`.
    pub latest_turn: Option<String>,
    pub turn_complete: bool,
    pub aborted_at: Option<SystemTime>,
    /// `task_complete.last_agent_message` — the reply text.
    pub last_agent_message: Option<String>,
    /// Highest `ordinal` seen on an `event_msg` line (the idle-confirm clock).
    pub last_event_ordinal: u64,
}

/// How a turn was accepted — maps to the step's `✓`/`⚠` completion line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompleteVia {
    HandoffAndIdleTurn,
    IdleTurnNoHandoff,
    HandoffLingerCap,
    BashLingerCap,
    CodexTaskComplete,
    QuietFallback,
}

/// The oracle's answer for one poll tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Working(Phase),
    Complete { text: String, via: CompleteVia },
    Failed(String),
}

/// Per-turn clock the caller owns across ticks (every hold is bounded, so each
/// field is "when did this state start"). `verdict` both reads and resets it.
#[derive(Debug, Default)]
pub struct OracleClock {
    /// Rule 4: the turn ended with no handoff file.
    pub end_turn_since: Option<Instant>,
    /// Rule 5: the handoff exists while the turn has not ended. Back-dated to
    /// the FILE's mtime, not to first observation.
    pub handoff_since: Option<Instant>,
    /// Rule 3: the confirm window.
    pub idle_since: Option<Instant>,
    /// The confirm window's claude anchor — `last_message_at` when it started;
    /// the window restarts when a new MESSAGE line lands (never on a file
    /// mtime: claude appends attachment/queue lines after an end_turn).
    pub idle_anchor: Option<SystemTime>,
    /// The confirm window's sub-agent anchor — `subagent_moved_at` when it
    /// started. Tracked SEPARATELY from `idle_anchor` (a grandchild's write is
    /// older than the parent's last message, so a max would hide it).
    pub idle_sub_anchor: Option<SystemTime>,
    /// The confirm window's codex anchor (`last_event_ordinal` at window start).
    pub idle_ordinal: u64,
    /// Rule 5b: the parent went idle with only background `Bash` ids pending.
    pub bash_only_since: Option<Instant>,
    /// [`QUEUE_ONLY_BOUND`]: the tail has been queue-operation-only since here.
    pub queue_only_since: Option<Instant>,
    /// `last_message_at` when `queue_only_since` started — a new message line
    /// means the notification WAS delivered, so the bound restarts.
    pub queue_anchor: Option<SystemTime>,
    /// The handoff nudge has been submitted (rule 4, once per turn).
    pub nudged: bool,
    /// codex: the latest turn was aborted and no new one started yet.
    pub aborted_since: Option<Instant>,
}

/// Per-tick inputs that are not in the scans themselves.
#[derive(Debug, Clone, Copy, Default)]
pub struct OracleOpts {
    /// `completed_turn_count` at submit time — a resumed transcript already
    /// holds the previous turns.
    pub baseline_turns: usize,
    /// codex rollout `ordinal` at submit time (the caller passes it to
    /// [`scan_codex`]; kept here so one struct carries the whole baseline).
    pub baseline_ordinal: u64,
    /// mtime of the handoff file, when it exists.
    pub handoff_mtime: Option<SystemTime>,
    /// Newest `subagents/*.jsonl` mtime — a grandchild still writing resets the
    /// confirm window (the only guard for nested sub-agents, design §8.11).
    pub subagent_moved_at: Option<SystemTime>,
}

/// Walk a claude session JSONL once: turn count/text, the tail state, the
/// pending/notified task sets and the api-error fail-fast. One pass, no I/O.
pub fn scan_claude(jsonl: &str) -> ClaudeScan {
    let mut s = ClaudeScan {
        api_error: otto_orchestrator::claude_pty::transcript_api_error(jsonl),
        ..Default::default()
    };
    // Tail state, folded forward: `tail_end_turn` is "the last message line was
    // an assistant end_turn and nothing since wakes the parent"; the queue flags
    // record whether the ONLY thing since was a queue-operation line.
    let mut tail_end_turn = false;
    let mut post_end_turn_queue_only = false;
    let mut saw_queue_after_end_turn = false;

    for (i, line) in jsonl.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // A partially-written trailing line (and any metadata shape we don't
        // parse) contributes nothing — exactly what `completed_turn_count` does.
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        // Legacy in-file sub-agent lines: never a parent turn, never a task.
        if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
            continue;
        }
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(parse_ts);
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let msg = v.get("message");
        let role = msg
            .and_then(|m| m.get("role"))
            .and_then(|r| r.as_str())
            .unwrap_or("");
        let is_message = matches!(role, "user" | "assistant");

        if is_message {
            if ts.is_some() {
                s.last_message_at = ts;
            }
            let end_turn = role == "assistant"
                && msg
                    .and_then(|m| m.get("stop_reason"))
                    .and_then(|r| r.as_str())
                    == Some("end_turn");
            let text = end_turn.then(|| assistant_text(msg)).flatten();
            match text {
                Some(t) => {
                    s.completed_turns += 1;
                    s.last_turn_text = Some(t);
                    tail_end_turn = true;
                    post_end_turn_queue_only = true;
                    saw_queue_after_end_turn = false;
                }
                None => {
                    tail_end_turn = false;
                    post_end_turn_queue_only = false;
                }
            }
        }

        // Launches / resumes (any non-sidechain line, message-bearing or not).
        if let Some(tur) = v.get("toolUseResult") {
            if tur.get("status").and_then(|x| x.as_str()) == Some("async_launched") {
                if let Some(id) = task_id(tur.get("agentId")) {
                    let desc = tur
                        .get("description")
                        .and_then(|d| d.as_str())
                        .map(|d| truncate_chars(d, 80))
                        .unwrap_or_else(|| "sub-agent".to_string());
                    push_pending(&mut s.pending, id, TaskKind::Agent, desc, i);
                }
            }
            if let Some(id) = task_id(tur.get("backgroundTaskId")) {
                let desc = tur
                    .get("command")
                    .and_then(|c| c.as_str())
                    .map(|c| truncate_chars(c, 80))
                    .unwrap_or_else(|| "background task".to_string());
                push_pending(&mut s.pending, id, TaskKind::Bash, desc, i);
            }
            // A `SendMessage` to a FINISHED agent resumes it — a later
            // notification follows, so the id is pending again.
            if let Some(id) = task_id(tur.get("resumedAgentId")) {
                s.notified.remove(&id);
                s.notified_at.remove(&id);
                push_pending(&mut s.pending, id, TaskKind::Agent, "resumed sub-agent".into(), i);
            }
        }

        // Notifications, wherever they ride (queue-operation line, user string
        // line, or embedded in a `tool_result` block) — matched on the RAW line.
        let notes = find_task_notifications(line);
        let is_queue_op = kind == "queue-operation";
        for (id, status) in &notes {
            s.pending.retain(|p| &p.id != id);
            s.notified.insert(id.clone(), status.clone());
            if let Some(t) = ts {
                s.notified_at.insert(id.clone(), t);
            }
        }
        if !notes.is_empty() {
            // A `remove`d notification is never delivered — it only clears the
            // task; the tail flag stays as it was.
            let removed =
                is_queue_op && v.get("operation").and_then(|o| o.as_str()) == Some("remove");
            if !removed {
                tail_end_turn = false;
                if !is_queue_op {
                    // A real wake-up line: the tail is no longer queue-only.
                    post_end_turn_queue_only = false;
                }
            }
        }
        if is_queue_op {
            saw_queue_after_end_turn = true;
        }
    }

    s.tail_is_assistant_end_turn = tail_end_turn;
    s.tail_evidence_is_queue_only = post_end_turn_queue_only && saw_queue_after_end_turn;
    s
}

/// Concatenated non-empty `type:"text"` blocks of an assistant message, or
/// `None` when it carries none (mirrors `completed_turn_text`'s filter).
fn assistant_text(msg: Option<&serde_json::Value>) -> Option<String> {
    let blocks = msg?.get("content")?.as_array()?;
    let mut text = String::new();
    for b in blocks {
        if b.get("type").and_then(|t| t.as_str()) != Some("text") {
            continue;
        }
        if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
            if !t.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(t);
            }
        }
    }
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// An opaque harness id: `[A-Za-z0-9_-]{1,64}`, never logged beyond the id.
fn task_id(v: Option<&serde_json::Value>) -> Option<String> {
    let s = v?.as_str()?.trim();
    valid_id(s).then(|| s.to_string())
}

fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// De-duplicating push: a re-launch of an id we already hold is ignored.
fn push_pending(
    pending: &mut Vec<PendingTask>,
    id: String,
    kind: TaskKind,
    description: String,
    since_line: usize,
) {
    if pending.iter().any(|p| p.id == id) {
        return;
    }
    pending.push(PendingTask { id, kind, description, since_line });
}

fn truncate_chars(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

fn parse_ts(s: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| SystemTime::from(d.with_timezone(&chrono::Utc)))
}

/// Every `<task-notification>` in one raw transcript line, as `(id, status)`.
/// Plain scanning (no regex dep): find the tag, then this notification's
/// `<task-id>`/`<status>` before the next one starts.
fn find_task_notifications(line: &str) -> Vec<(String, String)> {
    const NOTIF: &str = "<task-notification>";
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = line[pos..].find(NOTIF) {
        let start = pos + rel + NOTIF.len();
        let next = line[start..]
            .find(NOTIF)
            .map(|r| start + r)
            .unwrap_or(line.len());
        let seg = &line[start..next];
        pos = next.max(start);
        let Some((id, id_end)) = tag_value(seg, "task-id", 0) else {
            continue;
        };
        if !valid_id(&id) {
            continue;
        }
        let status = tag_value(seg, "status", id_end)
            .map(|(s, _)| s)
            .filter(|s| matches!(s.as_str(), "completed" | "failed" | "killed" | "stopped"))
            .unwrap_or_else(|| "completed".to_string());
        out.push((id, status));
    }
    out
}

/// `(value, end_offset)` of `<tag>…</tag>` in `seg` at/after `from`.
fn tag_value(seg: &str, tag: &str, from: usize) -> Option<(String, usize)> {
    if from >= seg.len() {
        return None;
    }
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let s = seg[from..].find(&open)? + from + open.len();
    let e = seg[s..].find(&close)? + s;
    Some((seg[s..e].to_string(), e + close.len()))
}

/// Walk a codex rollout past `after_ordinal`: the LATEST `task_started` turn,
/// whether its `task_complete` landed, and an abort of that same turn.
pub fn scan_codex(rollout: &str, after_ordinal: u64) -> CodexScan {
    let mut s = CodexScan::default();
    for line in rollout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let ordinal = v.get("ordinal").and_then(|o| o.as_u64()).unwrap_or(0);
        if ordinal <= after_ordinal {
            continue;
        }
        if v.get("type").and_then(|t| t.as_str()) != Some("event_msg") {
            continue;
        }
        s.last_event_ordinal = s.last_event_ordinal.max(ordinal);
        let Some(payload) = v.get("payload") else { continue };
        let turn_id = payload.get("turn_id").and_then(|t| t.as_str());
        match payload.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "task_started" => {
                s.latest_turn = turn_id.map(str::to_string);
                s.turn_complete = false;
                s.aborted_at = None;
            }
            "turn_aborted" if turn_id.is_some() && turn_id == s.latest_turn.as_deref() => {
                s.latest_turn = None;
                s.aborted_at = v
                    .get("timestamp")
                    .and_then(|t| t.as_str())
                    .and_then(parse_ts)
                    .or_else(|| Some(SystemTime::now()));
            }
            "task_complete" if turn_id.is_some() && turn_id == s.latest_turn.as_deref() => {
                s.turn_complete = true;
                s.last_agent_message = payload
                    .get("last_agent_message")
                    .and_then(|m| m.as_str())
                    .map(str::to_string);
            }
            _ => {}
        }
    }
    s
}

/// The rollout's highest `ordinal` (0 for a missing/empty file) — the submit
/// baseline, so a prior turn's `task_complete` can't be read as this one's.
pub fn codex_last_ordinal(rollout: &str) -> u64 {
    rollout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
        .filter_map(|v| v.get("ordinal").and_then(|o| o.as_u64()))
        .max()
        .unwrap_or(0)
}

/// The step's sub-agents for DISPLAY: description/`started_at` from
/// `<psid>/subagents/agent-<id>.meta.json`, status strictly from `scan`. Ids
/// without a meta file still appear. Never reads a child's transcript.
pub fn subagents(project_dir: &Path, psid: &str, scan: &ClaudeScan) -> Vec<SubagentInfo> {
    let dir = project_dir.join(psid).join("subagents");
    // id → (description, meta mtime)
    let mut meta: BTreeMap<String, (Option<String>, Option<SystemTime>)> = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(stem) = name.strip_suffix(".meta.json") else { continue };
            let Some(id) = stem.strip_prefix("agent-") else { continue };
            if !valid_id(id) {
                continue;
            }
            let desc = std::fs::read_to_string(e.path())
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                .and_then(|v| {
                    v.get("description")
                        .and_then(|d| d.as_str())
                        .map(|d| truncate_chars(d, 80))
                });
            let mtime = e.metadata().ok().and_then(|m| m.modified().ok());
            meta.insert(id.to_string(), (desc, mtime));
        }
    }
    let mut out: Vec<SubagentInfo> = Vec::new();
    let ids = scan
        .pending
        .iter()
        .map(|p| p.id.clone())
        .chain(scan.notified.keys().cloned());
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let m = meta.get(&id);
        let launched = scan.pending.iter().find(|p| p.id == id);
        let description = m
            .and_then(|(d, _)| d.clone())
            .or_else(|| launched.map(|p| p.description.clone()))
            .unwrap_or_else(|| "sub-agent".to_string());
        let status = if launched.is_some() {
            SubStatus::Running
        } else if scan.notified.get(&id).map(String::as_str) == Some("completed") {
            SubStatus::Done
        } else {
            SubStatus::Failed
        };
        out.push(SubagentInfo {
            id: id.clone(),
            description,
            status,
            started_at: m.and_then(|(_, t)| *t),
            finished_at: scan.notified_at.get(&id).copied(),
        });
    }
    // Oldest first; entries with no meta file sort last, then by id.
    out.sort_by(|a, b| match (a.started_at, b.started_at) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.id.cmp(&b.id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.id.cmp(&b.id),
    });
    out.truncate(40);
    out
}

/// Newest mtime over the main transcript, `subagents/*.jsonl` and
/// `tasks/*.output` — the stall clock. `None` when nothing exists yet
/// (never "no progress").
pub fn progress_stamp(
    main: &Path,
    subagent_dir: Option<&Path>,
    tasks_dir: Option<&Path>,
) -> Option<SystemTime> {
    let mut newest = std::fs::metadata(main).and_then(|m| m.modified()).ok();
    let mut scan_dir = |dir: &Path, ext: &str| {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if !e.file_name().to_string_lossy().ends_with(ext) {
                    continue;
                }
                if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                    if newest.is_none_or(|n| m > n) {
                        newest = Some(m);
                    }
                }
            }
        }
    };
    if let Some(d) = subagent_dir {
        scan_dir(d, ".jsonl");
    }
    if let Some(d) = tasks_dir {
        scan_dir(d, ".output");
    }
    newest
}

/// The oracle's per-tick decision (design §1.1 rules 1–6, in order).
pub fn verdict(
    provider: &str,
    claude: Option<&ClaudeScan>,
    codex: Option<&CodexScan>,
    handoff: Option<&str>,
    clock: &mut OracleClock,
    now: Instant,
    opts: &OracleOpts,
) -> Verdict {
    // The provider decides which scan is authoritative when a defensive caller
    // hands both.
    let (claude, codex) = match provider {
        "claude" => (claude, None),
        "codex" => (None, codex),
        _ => (claude, codex),
    };
    let handoff_text: Option<String> = handoff
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // Providers with no transcript at all (agy/custom): the handoff file is the
    // only positive signal; otherwise the caller's `quiet_done` channel decides.
    if claude.is_none() && codex.is_none() {
        return match handoff_text {
            Some(t) => Verdict::Complete { text: t, via: CompleteVia::HandoffAndIdleTurn },
            None => Verdict::Working(Phase::Working),
        };
    }

    // 1. A claude API error (wrong model / auth / quota) fails fast.
    if let Some(e) = claude.and_then(|c| c.api_error.clone()) {
        return Verdict::Failed(e);
    }

    // 2. Did the turn END natively, with nothing pending?
    let mut turn_ended = false;
    if let Some(c) = claude {
        // The 60s queue-only bound: an enqueued notification that never gets
        // delivered (a `remove`) must not hold the tail open forever.
        if c.tail_evidence_is_queue_only {
            if clock.queue_only_since.is_none() || clock.queue_anchor != c.last_message_at {
                clock.queue_only_since = Some(now);
                clock.queue_anchor = c.last_message_at;
            }
        } else {
            clock.queue_only_since = None;
            clock.queue_anchor = None;
        }
        let queue_expired = clock
            .queue_only_since
            .is_some_and(|t| now.saturating_duration_since(t) >= QUEUE_ONLY_BOUND);
        turn_ended = c.completed_turns > opts.baseline_turns
            && (c.tail_is_assistant_end_turn || queue_expired)
            && c.pending.is_empty();
    } else if let Some(x) = codex {
        if x.latest_turn.is_none() && x.aborted_at.is_some() {
            let since = *clock.aborted_since.get_or_insert(now);
            if now.saturating_duration_since(since) >= HANDOFF_MISSING_GRACE {
                return Verdict::Failed("codex turn aborted".into());
            }
        } else {
            clock.aborted_since = None;
        }
        turn_ended = x.turn_complete;
    }

    // 3. Turn ended WITH a handoff → confirm the transcript stays quiet.
    if turn_ended && handoff_text.is_some() {
        let anchor = claude.and_then(|c| c.last_message_at);
        let sub_anchor = opts.subagent_moved_at;
        let ordinal = codex.map(|x| x.last_event_ordinal).unwrap_or(0);
        if clock.idle_since.is_none()
            || clock.idle_anchor != anchor
            || clock.idle_sub_anchor != sub_anchor
            || clock.idle_ordinal != ordinal
        {
            clock.idle_since = Some(now);
            clock.idle_anchor = anchor;
            clock.idle_sub_anchor = sub_anchor;
            clock.idle_ordinal = ordinal;
        }
        let elapsed = now.saturating_duration_since(clock.idle_since.unwrap_or(now));
        if elapsed >= IDLE_CONFIRM {
            let via = if codex.is_some() {
                CompleteVia::CodexTaskComplete
            } else {
                CompleteVia::HandoffAndIdleTurn
            };
            return Verdict::Complete { text: handoff_text.unwrap_or_default(), via };
        }
        return Verdict::Working(Phase::IdleConfirming { left: IDLE_CONFIRM - elapsed });
    }
    clock.idle_since = None;

    // 4. Turn ended with NO handoff → nudge once, then accept the final reply.
    if turn_ended {
        let since = *clock.end_turn_since.get_or_insert(now);
        let elapsed = now.saturating_duration_since(since);
        if elapsed >= HANDOFF_MISSING_GRACE {
            let text = claude
                .and_then(|c| c.last_turn_text.clone())
                .or_else(|| codex.and_then(|x| x.last_agent_message.clone()))
                .unwrap_or_default();
            return Verdict::Complete { text, via: CompleteVia::IdleTurnNoHandoff };
        }
        return Verdict::Working(Phase::HandoffMissingGrace {
            left: HANDOFF_MISSING_GRACE - elapsed,
        });
    }
    clock.end_turn_since = None;

    // 5b (before 5). The parent is idle with only BACKGROUND tasks left — a dev
    // server it forgot to stop. The clock starts when that state is first seen,
    // never at the task's launch (a 20-min test run is not cut short).
    if let Some(c) = claude {
        let bash_only = c.tail_is_assistant_end_turn
            && !c.pending.is_empty()
            && c.pending.iter().all(|p| p.kind == TaskKind::Bash);
        if bash_only {
            let since = *clock.bash_only_since.get_or_insert(now);
            let elapsed = now.saturating_duration_since(since);
            if elapsed >= BASH_LINGER_CAP {
                let text = handoff_text
                    .clone()
                    .or_else(|| c.last_turn_text.clone())
                    .unwrap_or_default();
                return Verdict::Complete { text, via: CompleteVia::BashLingerCap };
            }
            return Verdict::Working(Phase::BashLinger {
                pending: c.pending.len(),
                left: BASH_LINGER_CAP - elapsed,
            });
        }
        clock.bash_only_since = None;
    }

    // 5. The handoff says "done" while the turn is still open → wait for the
    // stragglers, bounded.
    if let Some(text) = handoff_text {
        let since = *clock.handoff_since.get_or_insert_with(|| {
            // Back-date to the file's own mtime: a handoff written before the
            // daemon looked must not restart the 15-minute cap.
            let age = opts
                .handoff_mtime
                .and_then(|m| SystemTime::now().duration_since(m).ok())
                .unwrap_or_default();
            now.checked_sub(age).unwrap_or(now)
        });
        if now.saturating_duration_since(since) >= HANDOFF_LINGER_CAP {
            return Verdict::Complete { text, via: CompleteVia::HandoffLingerCap };
        }
        let pending = claude.map(|c| c.pending.len()).unwrap_or(0);
        return Verdict::Working(Phase::HandoffWrittenWaiting { pending });
    }
    clock.handoff_since = None;

    // 6. Still working. "Done" means the task actually COMPLETED — `subagents()`
    // reports a failed/killed/stopped child as `Failed` and the run view's chip
    // counts only the completed ones, so counting them here too would make the
    // 🧩 line and the chip disagree.
    let running = claude.map(|c| c.pending.len()).unwrap_or(0);
    let done = claude
        .map(|c| c.notified.values().filter(|s| s.as_str() == "completed").count())
        .unwrap_or(0);
    if running > 0 || done > 0 {
        return Verdict::Working(Phase::Subagents { running, done });
    }
    Verdict::Working(Phase::Working)
}

/// The step-log line for a phase (design §4, verbatim). `provider` only feeds
/// `Booting`; the countdown phases carry their TOTAL, because they are logged
/// once on entry (see the emission rule in `run_node_agent`).
pub fn phase_line(p: &Phase, provider: &str) -> String {
    match p {
        Phase::Booting => format!("⏳ starting {provider} session"),
        Phase::PromptAccepted => "✉ prompt accepted".to_string(),
        Phase::Working => "⚙ working".to_string(),
        Phase::Subagents { running, done } => {
            format!("🧩 sub-agents: {running} running · {done} done")
        }
        Phase::IdleConfirming { .. } => format!(
            "⏸ agent idle — confirming completion ({}s)",
            IDLE_CONFIRM.as_secs()
        ),
        // Nothing pending is the NORMAL tail of a step: the handoff is on disk
        // and the model is still producing its closing text. Only a handoff
        // written while tasks are still in flight is the protocol violation the
        // ⚠ form (and the docs) describe.
        Phase::HandoffWrittenWaiting { pending: 0 } => format!(
            "📄 handoff written — waiting for the turn to end (up to {}m)",
            HANDOFF_LINGER_CAP.as_secs() / 60
        ),
        Phase::HandoffWrittenWaiting { pending } => format!(
            "⚠ handoff written but {pending} tasks still pending — waiting (up to {}m)",
            HANDOFF_LINGER_CAP.as_secs() / 60
        ),
        Phase::HandoffMissingGrace { .. } => format!(
            "⏸ handoff file missing — waiting up to {}s for the agent's final reply",
            HANDOFF_MISSING_GRACE.as_secs()
        ),
        Phase::BashLinger { .. } => format!(
            "⏸ background task still running — waiting up to {}m",
            BASH_LINGER_CAP.as_secs() / 60
        ),
    }
}

/// Phase identity for the emission rule — ignores the countdown's `left`, so a
/// ticking window logs once instead of once per second.
pub fn phase_key(p: &Phase) -> (u8, usize, usize) {
    match p {
        Phase::Booting => (0, 0, 0),
        Phase::PromptAccepted => (1, 0, 0),
        Phase::Working => (2, 0, 0),
        Phase::Subagents { running, done } => (3, *running, *done),
        Phase::IdleConfirming { .. } => (4, 0, 0),
        Phase::HandoffWrittenWaiting { pending } => (5, *pending, 0),
        Phase::HandoffMissingGrace { .. } => (6, 0, 0),
        Phase::BashLinger { pending, .. } => (7, *pending, 0),
    }
}

/// The stall trip is consulted ONLY while the agent looks like it is working —
/// every other phase carries its own cap, and tripping there would kill a step
/// that already wrote its handoff.
pub fn stall_trip_applies(phase: &Phase) -> bool {
    matches!(phase, Phase::Working | Phase::Subagents { .. })
}

/// Whether the no-progress trip fires. `since_progress` already accounts for
/// the child files (it comes from [`progress_stamp`]), so a sweep whose
/// sub-agents are writing never trips even with a quiet parent.
pub fn stall_trip_fires(
    phase: &Phase,
    pending: usize,
    since_progress: Duration,
    stuck_after: Duration,
) -> bool {
    let _ = pending; // reported in the error text; the clock already covers it
    stall_trip_applies(phase) && since_progress >= stuck_after
}

/// True for the phase lines the log cap may evict (never `▶ ✓ ⚠ ↻ ✗`, retry,
/// edge or persist lines).
pub fn is_phase_line(s: &str) -> bool {
    ["⏳", "✉", "⚙", "🧩", "⏸", "📄"]
        .iter()
        .any(|g| s.starts_with(g))
}

/// Cap a node's log vector at `cap`, evicting ONLY phase lines, oldest first.
/// Stops when none is left, even above the cap — a run's decisions are never
/// dropped to make room for progress chatter.
pub fn cap_node_logs(logs: &mut Vec<String>, cap: usize) {
    while logs.len() > cap {
        let Some(i) = logs.iter().position(|l| is_phase_line(l)) else {
            return;
        };
        logs.remove(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Anonymised copies of the real line shapes (design §0).
    mod fixtures {
        pub fn user(ts: &str, text: &str) -> String {
            format!(
                r#"{{"type":"user","timestamp":"{ts}","message":{{"role":"user","content":{}}}}}"#,
                serde_json::to_string(text).unwrap()
            )
        }
        pub fn end_turn(ts: &str, text: &str) -> String {
            format!(
                r#"{{"type":"assistant","timestamp":"{ts}","message":{{"role":"assistant","stop_reason":"end_turn","content":[{{"type":"text","text":{}}}]}}}}"#,
                serde_json::to_string(text).unwrap()
            )
        }
        pub fn tool_use(ts: &str) -> String {
            format!(
                r#"{{"type":"assistant","timestamp":"{ts}","message":{{"role":"assistant","stop_reason":"tool_use","content":[{{"type":"text","text":"reading"}}]}}}}"#
            )
        }
        /// `Agent` tool launch — async even without `run_in_background`.
        pub fn launch(ts: &str, id: &str, desc: &str) -> String {
            format!(
                r#"{{"type":"user","timestamp":"{ts}","message":{{"role":"user","content":[{{"type":"tool_result","content":"Async agent launched successfully"}}]}},"toolUseResult":{{"isAsync":true,"status":"async_launched","agentId":"{id}","description":"{desc}"}}}}"#
            )
        }
        pub fn background(ts: &str, id: &str, cmd: &str) -> String {
            format!(
                r#"{{"type":"user","timestamp":"{ts}","message":{{"role":"user","content":[{{"type":"tool_result","content":"Command running in background with ID: {id}"}}]}},"toolUseResult":{{"backgroundTaskId":"{id}","command":"{cmd}"}}}}"#
            )
        }
        pub fn resume(ts: &str, id: &str) -> String {
            format!(
                r#"{{"type":"user","timestamp":"{ts}","message":{{"role":"user","content":[{{"type":"tool_result","content":"Message delivered"}}]}},"toolUseResult":{{"success":true,"message":"ok","pin":false,"resumedAgentId":"{id}"}}}}"#
            )
        }
        pub fn notif_body(id: &str, status: &str) -> String {
            format!(
                "<task-notification><task-id>{id}</task-id><tool-use-id>tu_{id}</tool-use-id>\
                 <output-file>/tmp/tasks/{id}.output</output-file><status>{status}</status></task-notification>"
            )
        }
        /// The common shape: a `role:"user"` line whose content IS the notification.
        pub fn notif_user(ts: &str, id: &str, status: &str) -> String {
            user(ts, &notif_body(id, status))
        }
        /// Mid-tool-loop shape: the notification embedded in a `tool_result` block.
        pub fn notif_tool_result(ts: &str, id: &str, status: &str) -> String {
            format!(
                r#"{{"type":"user","timestamp":"{ts}","message":{{"role":"user","content":[{{"type":"tool_result","content":{}}}]}}}}"#,
                serde_json::to_string(&notif_body(id, status)).unwrap()
            )
        }
        /// Harness bookkeeping written BEFORE the parent is woken.
        pub fn queue_op(ts: &str, op: &str, id: &str, status: &str) -> String {
            format!(
                r#"{{"type":"queue-operation","timestamp":"{ts}","operation":"{op}","content":{}}}"#,
                serde_json::to_string(&notif_body(id, status)).unwrap()
            )
        }
        pub fn sidechain_end_turn(ts: &str, text: &str) -> String {
            format!(
                r#"{{"type":"assistant","isSidechain":true,"timestamp":"{ts}","message":{{"role":"assistant","stop_reason":"end_turn","content":[{{"type":"text","text":"{text}"}}]}}}}"#
            )
        }
        pub fn codex(ordinal: u64, ts: &str, payload: &str) -> String {
            format!(r#"{{"timestamp":"{ts}","ordinal":{ordinal},"type":"event_msg","payload":{payload}}}"#)
        }
    }
    use fixtures::*;

    fn opts() -> OracleOpts {
        OracleOpts::default()
    }

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn early_end_turn_with_pending_async_is_not_complete() {
        // The exact false positive: 4 launches, then the parent's "waiting on
        // the four sweep agents" end_turn.
        let mut jsonl = String::new();
        for (i, id) in ["aa", "ab", "ac", "ad"].iter().enumerate() {
            jsonl.push_str(&launch(
                &format!("2026-09-12T08:07:{:02}Z", 17 + i),
                id,
                "Sweep diff chunk",
            ));
            jsonl.push('\n');
        }
        jsonl.push_str(&end_turn(
            "2026-09-12T08:12:30Z",
            "My own pass is complete. Waiting on the four sweep agents",
        ));
        let scan = scan_claude(&jsonl);
        assert_eq!(scan.pending.len(), 4);
        assert_eq!(scan.completed_turns, 1);
        assert!(scan.tail_is_assistant_end_turn);
        let mut clock = OracleClock::default();
        assert_eq!(
            verdict("claude", Some(&scan), None, None, &mut clock, t0(), &opts()),
            Verdict::Working(Phase::Subagents { running: 4, done: 0 })
        );
    }

    #[test]
    fn notification_clears_pending_one_by_one() {
        let ids = ["aa", "ab", "ac", "ad"];
        let mut jsonl = String::new();
        for id in ids {
            jsonl.push_str(&launch("2026-09-12T08:07:17Z", id, "Sweep"));
            jsonl.push('\n');
        }
        jsonl.push_str(&end_turn("2026-09-12T08:12:30Z", "waiting on the sweeps"));
        jsonl.push('\n');
        for (i, id) in ids.iter().enumerate() {
            jsonl.push_str(&notif_user(&format!("2026-09-12T08:1{i}:00Z"), id, "completed"));
            jsonl.push('\n');
            let scan = scan_claude(&jsonl);
            assert_eq!(scan.pending.len(), 3 - i, "after {id}");
            assert_eq!(scan.notified.len(), i + 1);
            // A notification WAKES the parent — the tail is no longer end_turn.
            assert!(!scan.tail_is_assistant_end_turn);
            jsonl.push_str(&end_turn(
                &format!("2026-09-12T08:1{i}:05Z"),
                "chunk came back",
            ));
            jsonl.push('\n');
        }
        let scan = scan_claude(&jsonl);
        assert!(scan.pending.is_empty());
        assert!(scan.tail_is_assistant_end_turn);
        assert_eq!(scan.notified.len(), 4);
    }

    #[test]
    fn notification_wakes_the_parent_so_tail_is_not_end_turn() {
        let jsonl = format!(
            "{}\n{}\n{}\n",
            launch("2026-09-12T08:07:17Z", "aa", "Sweep"),
            end_turn("2026-09-12T08:12:30Z", "waiting"),
            notif_user("2026-09-12T08:12:32Z", "aa", "completed"),
        );
        let scan = scan_claude(&jsonl);
        assert!(!scan.tail_is_assistant_end_turn);
        assert!(scan.pending.is_empty());
        // Turn not ended → not complete even with a handoff on disk.
        let mut clock = OracleClock::default();
        assert!(matches!(
            verdict("claude", Some(&scan), None, Some("done"), &mut clock, t0(), &opts()),
            Verdict::Working(Phase::HandoffWrittenWaiting { pending: 0 })
        ));
    }

    #[test]
    fn notification_embedded_in_tool_result_clears_pending() {
        let jsonl = format!(
            "{}\n{}\n",
            launch("2026-09-12T08:07:17Z", "ac", "Sweep"),
            notif_tool_result("2026-09-12T08:16:05Z", "ac", "completed"),
        );
        let scan = scan_claude(&jsonl);
        assert!(scan.pending.is_empty());
        assert_eq!(scan.notified.get("ac").map(String::as_str), Some("completed"));
    }

    #[test]
    fn queue_operation_enqueue_after_end_turn_means_not_tail() {
        let jsonl = format!(
            "{}\n{}\n{}\n",
            launch("2026-09-12T08:07:17Z", "aa", "Sweep"),
            end_turn("2026-09-12T08:12:30Z", "waiting"),
            queue_op("2026-09-12T08:12:31Z", "enqueue", "aa", "completed"),
        );
        let scan = scan_claude(&jsonl);
        assert!(!scan.tail_is_assistant_end_turn);
        assert!(scan.tail_evidence_is_queue_only);
        // The 60s bound: an enqueued-but-never-delivered notification stops
        // holding the tail open.
        let mut clock = OracleClock::default();
        let t = t0();
        assert!(matches!(
            verdict("claude", Some(&scan), None, Some("handoff"), &mut clock, t, &opts()),
            Verdict::Working(Phase::HandoffWrittenWaiting { .. })
        ));
        let later = t + QUEUE_ONLY_BOUND + Duration::from_secs(1);
        assert!(matches!(
            verdict("claude", Some(&scan), None, Some("handoff"), &mut clock, later, &opts()),
            Verdict::Working(Phase::IdleConfirming { .. })
        ));
    }

    #[test]
    fn completed_child_with_null_stop_reason_is_done() {
        let dir = tempfile::tempdir().unwrap();
        let psid = "sess-1";
        let subs = dir.path().join(psid).join("subagents");
        std::fs::create_dir_all(&subs).unwrap();
        std::fs::write(
            subs.join("agent-aa.meta.json"),
            r#"{"agentType":"general","description":"Sweep diff chunk aa","toolUseId":"tu_aa","spawnDepth":1}"#,
        )
        .unwrap();
        // The child's OWN transcript ends mid-turn — never a completion signal.
        std::fs::write(
            subs.join("agent-aa.jsonl"),
            r#"{"message":{"role":"assistant","stop_reason":null,"content":[{"type":"text","text":"…"}]}}"#,
        )
        .unwrap();
        let jsonl = format!(
            "{}\n{}\n{}\n",
            launch("2026-09-12T08:07:17Z", "aa", "Sweep diff chunk aa"),
            launch("2026-09-12T08:07:18Z", "zz", "Sweep with no files at all"),
            notif_user("2026-09-12T08:12:32Z", "aa", "completed"),
        );
        let scan = scan_claude(&jsonl);
        let subs = subagents(dir.path(), psid, &scan);
        let aa = subs.iter().find(|s| s.id == "aa").expect("aa present");
        assert_eq!(aa.status, SubStatus::Done);
        assert_eq!(aa.description, "Sweep diff chunk aa");
        assert!(aa.finished_at.is_some());
        // An id with no files at all still shows, from the launch line.
        let zz = subs.iter().find(|s| s.id == "zz").expect("zz present");
        assert_eq!(zz.status, SubStatus::Running);
        assert_eq!(zz.description, "Sweep with no files at all");
        assert!(zz.started_at.is_none());
    }

    #[test]
    fn send_message_resume_reopens_pending() {
        let base = format!(
            "{}\n{}\n",
            launch("2026-09-12T08:07:17Z", "aa", "Sweep"),
            notif_user("2026-09-12T08:12:32Z", "aa", "completed"),
        );
        assert!(scan_claude(&base).pending.is_empty());
        let resumed = format!("{base}{}\n", resume("2026-09-12T08:13:00Z", "aa"));
        let scan = scan_claude(&resumed);
        assert_eq!(scan.pending.len(), 1);
        assert!(!scan.notified.contains_key("aa"));
        let done = format!("{resumed}{}\n", notif_user("2026-09-12T08:14:00Z", "aa", "completed"));
        assert!(scan_claude(&done).pending.is_empty());
    }

    #[test]
    fn background_bash_counts_as_pending() {
        let jsonl = format!(
            "{}\n",
            background("2026-09-12T08:07:17Z", "b5gvqf675", "npm run dev")
        );
        let scan = scan_claude(&jsonl);
        assert_eq!(scan.pending.len(), 1);
        assert_eq!(scan.pending[0].kind, TaskKind::Bash);
        assert_eq!(scan.pending[0].description, "npm run dev");
    }

    #[test]
    fn bash_only_pending_completes_after_linger_cap() {
        // The clock starts when the parent goes IDLE with only Bash ids left —
        // not at the launch (a 20-min test run it is working alongside is safe).
        let working = format!(
            "{}\n{}\n",
            background("2026-09-12T08:00:00Z", "b5", "npm run dev"),
            tool_use("2026-09-12T08:20:00Z"),
        );
        let mut clock = OracleClock::default();
        let t = t0();
        let scan = scan_claude(&working);
        assert!(matches!(
            verdict("claude", Some(&scan), None, None, &mut clock, t, &opts()),
            Verdict::Working(Phase::Subagents { running: 1, done: 0 })
        ));
        assert!(clock.bash_only_since.is_none());

        let idle = format!("{working}{}\n", end_turn("2026-09-12T08:21:00Z", "all done"));
        let scan = scan_claude(&idle);
        assert!(matches!(
            verdict("claude", Some(&scan), None, None, &mut clock, t, &opts()),
            Verdict::Working(Phase::BashLinger { pending: 1, .. })
        ));
        // A notification waking the parent resets the clock.
        let woken = format!("{idle}{}\n", notif_user("2026-09-12T08:22:00Z", "other", "completed"));
        let woken_scan = scan_claude(&woken);
        let _ = verdict("claude", Some(&woken_scan), None, None, &mut clock, t, &opts());
        assert!(clock.bash_only_since.is_none());

        // …and at the cap the step moves on, with and without a handoff.
        let mut clock = OracleClock::default();
        let scan = scan_claude(&idle);
        let _ = verdict("claude", Some(&scan), None, None, &mut clock, t, &opts());
        let late = t + BASH_LINGER_CAP + Duration::from_secs(1);
        assert_eq!(
            verdict("claude", Some(&scan), None, None, &mut clock, late, &opts()),
            Verdict::Complete { text: "all done".into(), via: CompleteVia::BashLingerCap }
        );
        let mut clock = OracleClock::default();
        let _ = verdict("claude", Some(&scan), None, Some("handoff"), &mut clock, t, &opts());
        assert_eq!(
            verdict("claude", Some(&scan), None, Some("handoff"), &mut clock, late, &opts()),
            Verdict::Complete { text: "handoff".into(), via: CompleteVia::BashLingerCap }
        );
    }

    #[test]
    fn legacy_sidechain_lines_never_count() {
        let jsonl = format!(
            "{}\n{}\n",
            sidechain_end_turn("2026-09-12T08:00:00Z", "child reply"),
            tool_use("2026-09-12T08:00:01Z"),
        );
        let scan = scan_claude(&jsonl);
        assert_eq!(scan.completed_turns, 0);
        assert!(!scan.tail_is_assistant_end_turn);
        // …and a sidechain line never touches the pending set either.
        let with_launch = r#"{"isSidechain":true,"type":"user","timestamp":"2026-09-12T08:00:02Z","message":{"role":"user","content":"x"},"toolUseResult":{"status":"async_launched","agentId":"gc","description":"grandchild"}}"#;
        assert!(scan_claude(with_launch).pending.is_empty());
    }

    #[test]
    fn done_file_before_children_waits_then_linger_cap() {
        let jsonl = format!(
            "{}\n{}\n{}\n",
            launch("2026-09-12T08:07:17Z", "aa", "Sweep"),
            launch("2026-09-12T08:07:18Z", "ab", "Sweep"),
            tool_use("2026-09-12T08:08:00Z"),
        );
        let scan = scan_claude(&jsonl);
        let mut clock = OracleClock::default();
        let t = t0();
        assert_eq!(
            verdict("claude", Some(&scan), None, Some("handoff text"), &mut clock, t, &opts()),
            Verdict::Working(Phase::HandoffWrittenWaiting { pending: 2 })
        );
        let late = t + HANDOFF_LINGER_CAP + Duration::from_secs(1);
        assert_eq!(
            verdict("claude", Some(&scan), None, Some("handoff text"), &mut clock, late, &opts()),
            Verdict::Complete { text: "handoff text".into(), via: CompleteVia::HandoffLingerCap }
        );
    }

    #[test]
    fn done_file_missing_accepts_after_grace() {
        let jsonl = format!("{}\n", end_turn("2026-09-12T08:16:42Z", "Review complete."));
        let scan = scan_claude(&jsonl);
        let mut clock = OracleClock::default();
        let t = t0();
        assert!(matches!(
            verdict("claude", Some(&scan), None, None, &mut clock, t, &opts()),
            Verdict::Working(Phase::HandoffMissingGrace { .. })
        ));
        let late = t + HANDOFF_MISSING_GRACE;
        assert_eq!(
            verdict("claude", Some(&scan), None, None, &mut clock, late, &opts()),
            Verdict::Complete { text: "Review complete.".into(), via: CompleteVia::IdleTurnNoHandoff }
        );
    }

    #[test]
    fn idle_confirm_keys_on_last_message_timestamp_not_file_mtime() {
        let jsonl = format!("{}\n", end_turn("2026-09-12T08:16:42Z", "done"));
        let scan = scan_claude(&jsonl);
        let mut clock = OracleClock::default();
        let t = t0();
        assert!(matches!(
            verdict("claude", Some(&scan), None, Some("handoff"), &mut clock, t, &opts()),
            Verdict::Working(Phase::IdleConfirming { .. })
        ));
        // A non-message line appended (attachment / queue bookkeeping) does not
        // move `last_message_at`, so the window keeps running down.
        let plus = format!(
            "{jsonl}{}\n",
            r#"{"type":"attachment","timestamp":"2026-09-12T08:16:50Z","content":"x"}"#
        );
        let scan2 = scan_claude(&plus);
        assert_eq!(scan2.last_message_at, scan.last_message_at);
        let late = t + IDLE_CONFIRM;
        assert_eq!(
            verdict("claude", Some(&scan2), None, Some("handoff"), &mut clock, late, &opts()),
            Verdict::Complete { text: "handoff".into(), via: CompleteVia::HandoffAndIdleTurn }
        );
    }

    #[test]
    fn idle_confirm_resets_when_a_subagent_file_moves() {
        let jsonl = format!("{}\n", end_turn("2026-09-12T08:16:42Z", "done"));
        let scan = scan_claude(&jsonl);
        let mut clock = OracleClock::default();
        let t = t0();
        let mut o = opts();
        o.subagent_moved_at = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000));
        assert!(matches!(
            verdict("claude", Some(&scan), None, Some("handoff"), &mut clock, t, &o),
            Verdict::Working(Phase::IdleConfirming { .. })
        ));
        // A grandchild wrote to its jsonl → the window restarts.
        o.subagent_moved_at = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_010));
        let late = t + IDLE_CONFIRM;
        assert!(matches!(
            verdict("claude", Some(&scan), None, Some("handoff"), &mut clock, late, &o),
            Verdict::Working(Phase::IdleConfirming { .. })
        ));
        // …and only then does the quiet window complete it.
        let later = late + IDLE_CONFIRM;
        assert!(matches!(
            verdict("claude", Some(&scan), None, Some("handoff"), &mut clock, later, &o),
            Verdict::Complete { via: CompleteVia::HandoffAndIdleTurn, .. }
        ));
    }

    #[test]
    fn progress_stamp_includes_subagent_and_task_files() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main.jsonl");
        std::fs::write(&main, "{}").unwrap();
        assert!(progress_stamp(&main, None, None).is_some());
        let main_m = std::fs::metadata(&main).unwrap().modified().unwrap();
        let subs = dir.path().join("subagents");
        let tasks = dir.path().join("tasks");
        std::fs::create_dir_all(&subs).unwrap();
        std::fs::create_dir_all(&tasks).unwrap();
        // A sub-agent writing while the parent is silent IS progress.
        let child = subs.join("agent-aa.jsonl");
        std::fs::write(&child, "{}").unwrap();
        let out = tasks.join("b5gvqf675.output");
        std::fs::write(&out, "listening on :5173").unwrap();
        let child_m = std::fs::metadata(&child).unwrap().modified().unwrap();
        let out_m = std::fs::metadata(&out).unwrap().modified().unwrap();
        let stamp = progress_stamp(&main, Some(&subs), Some(&tasks)).unwrap();
        assert_eq!(stamp, main_m.max(child_m).max(out_m));
        assert!(stamp >= main_m);
        // Files of other extensions in those dirs contribute nothing.
        std::fs::write(subs.join("agent-aa.meta.json"), "{}").unwrap();
        assert_eq!(progress_stamp(&main, Some(&subs), Some(&tasks)).unwrap(), stamp);
        // A missing dir is "no contribution", never an error.
        assert!(progress_stamp(&main, Some(&dir.path().join("nope")), None).is_some());
        // Nothing at all → None (never "no progress").
        assert!(progress_stamp(&dir.path().join("gone.jsonl"), None, None).is_none());
    }

    #[test]
    fn codex_task_complete_matches_latest_turn() {
        let rollout = format!(
            "{}\n{}\n",
            codex(10, "2026-09-12T08:00:00Z", r#"{"type":"task_started","turn_id":"T1"}"#),
            codex(
                20,
                "2026-09-12T08:05:00Z",
                r#"{"type":"task_complete","turn_id":"T1","last_agent_message":"all done"}"#
            ),
        );
        let scan = scan_codex(&rollout, 0);
        assert!(scan.turn_complete);
        assert_eq!(scan.last_agent_message.as_deref(), Some("all done"));
        assert_eq!(scan.last_event_ordinal, 20);
        assert_eq!(codex_last_ordinal(&rollout), 20);
        assert_eq!(codex_last_ordinal(""), 0);

        let mut clock = OracleClock::default();
        let t = t0();
        assert!(matches!(
            verdict("codex", None, Some(&scan), Some("handoff"), &mut clock, t, &opts()),
            Verdict::Working(Phase::IdleConfirming { .. })
        ));
        assert_eq!(
            verdict("codex", None, Some(&scan), Some("handoff"), &mut clock, t + IDLE_CONFIRM, &opts()),
            Verdict::Complete { text: "handoff".into(), via: CompleteVia::CodexTaskComplete }
        );
    }

    #[test]
    fn codex_aborted_turn_then_new_turn_completes() {
        let aborted = format!(
            "{}\n{}\n",
            codex(10, "2026-09-12T08:00:00Z", r#"{"type":"task_started","turn_id":"T1"}"#),
            codex(11, "2026-09-12T08:01:00Z", r#"{"type":"turn_aborted","turn_id":"T1"}"#),
        );
        let scan = scan_codex(&aborted, 0);
        assert!(scan.latest_turn.is_none());
        assert!(scan.aborted_at.is_some());
        let mut clock = OracleClock::default();
        let t = t0();
        let _ = verdict("codex", None, Some(&scan), None, &mut clock, t, &opts());
        assert_eq!(
            verdict("codex", None, Some(&scan), None, &mut clock, t + HANDOFF_MISSING_GRACE, &opts()),
            Verdict::Failed("codex turn aborted".into())
        );
        // A NEW turn after the abort completes normally.
        let restarted = format!(
            "{aborted}{}\n{}\n",
            codex(12, "2026-09-12T08:02:00Z", r#"{"type":"task_started","turn_id":"T2"}"#),
            codex(
                13,
                "2026-09-12T08:03:00Z",
                r#"{"type":"task_complete","turn_id":"T2","last_agent_message":"second"}"#
            ),
        );
        let scan = scan_codex(&restarted, 0);
        assert!(scan.turn_complete);
        assert_eq!(scan.latest_turn.as_deref(), Some("T2"));
    }

    #[test]
    fn codex_task_complete_of_prior_turn_ignored() {
        // The baseline skips the PREVIOUS turn entirely (a resumed rollout).
        let rollout = format!(
            "{}\n{}\n{}\n",
            codex(1, "2026-09-12T07:00:00Z", r#"{"type":"task_started","turn_id":"T0"}"#),
            codex(
                2,
                "2026-09-12T07:01:00Z",
                r#"{"type":"task_complete","turn_id":"T0","last_agent_message":"old"}"#
            ),
            codex(3, "2026-09-12T08:00:00Z", r#"{"type":"task_started","turn_id":"T1"}"#),
        );
        let scan = scan_codex(&rollout, 2);
        assert!(!scan.turn_complete);
        assert_eq!(scan.latest_turn.as_deref(), Some("T1"));
        // A stale complete for a turn that is no longer the latest is ignored.
        let stale = format!(
            "{rollout}{}\n",
            codex(
                4,
                "2026-09-12T08:00:10Z",
                r#"{"type":"task_complete","turn_id":"T0","last_agent_message":"old"}"#
            ),
        );
        assert!(!scan_codex(&stale, 2).turn_complete);
    }

    #[test]
    fn partial_trailing_line_is_skipped() {
        let jsonl = format!(
            "{}\n{}",
            end_turn("2026-09-12T08:16:42Z", "done"),
            r#"{"type":"assistant","timestamp":"2026-09-12T08:16:"#
        );
        let scan = scan_claude(&jsonl);
        assert_eq!(scan.completed_turns, 1);
        assert!(scan.tail_is_assistant_end_turn);
    }

    #[test]
    fn cap_evicts_only_phase_lines_oldest_first() {
        let mut logs: Vec<String> = vec!["▶ agent_prompt started".into()];
        for i in 0..300 {
            logs.push(format!("🧩 sub-agents: {i} running · 0 done"));
        }
        logs.push("⏸ agent idle — confirming completion (20s)".into());
        logs.push("⚠ handoff written but 3 tasks still pending — waiting (up to 15m)".into());
        logs.push("↻ retry 2/5 in 23s (provider overloaded: 529)".into());
        logs.push("✓ step complete (handoff + idle turn)".into());
        cap_node_logs(&mut logs, 200);
        assert_eq!(logs.len(), 200);
        assert_eq!(logs[0], "▶ agent_prompt started");
        assert!(logs.iter().any(|l| l.starts_with('↻')));
        assert!(logs.iter().any(|l| l.starts_with('⚠')));
        assert_eq!(logs.last().unwrap(), "✓ step complete (handoff + idle turn)");
        // The OLDEST 🧩 lines went first.
        assert!(!logs.iter().any(|l| l == "🧩 sub-agents: 0 running · 0 done"));
        assert!(logs.iter().any(|l| l == "🧩 sub-agents: 299 running · 0 done"));
        // Below the cap nothing is evicted; above it with no phase lines left,
        // the decision lines all survive.
        let mut only_decisions: Vec<String> = (0..10).map(|i| format!("✓ {i}")).collect();
        cap_node_logs(&mut only_decisions, 2);
        assert_eq!(only_decisions.len(), 10);
    }

    #[test]
    fn other_providers_complete_on_handoff_only() {
        let mut clock = OracleClock::default();
        assert_eq!(
            verdict("agy", None, None, Some(" reply "), &mut clock, t0(), &opts()),
            Verdict::Complete { text: "reply".into(), via: CompleteVia::HandoffAndIdleTurn }
        );
        assert_eq!(
            verdict("agy", None, None, None, &mut clock, t0(), &opts()),
            Verdict::Working(Phase::Working)
        );
    }

    #[test]
    fn phase_lines_are_the_documented_texts() {
        assert_eq!(phase_line(&Phase::Booting, "claude"), "⏳ starting claude session");
        assert_eq!(phase_line(&Phase::PromptAccepted, "claude"), "✉ prompt accepted");
        assert_eq!(phase_line(&Phase::Working, "claude"), "⚙ working");
        assert_eq!(
            phase_line(&Phase::Subagents { running: 2, done: 1 }, "claude"),
            "🧩 sub-agents: 2 running · 1 done"
        );
        assert_eq!(
            phase_line(&Phase::IdleConfirming { left: Duration::from_secs(12) }, "claude"),
            "⏸ agent idle — confirming completion (20s)"
        );
        assert_eq!(
            phase_line(&Phase::HandoffWrittenWaiting { pending: 3 }, "claude"),
            "⚠ handoff written but 3 tasks still pending — waiting (up to 15m)"
        );
        // …but a step simply finishing its turn is not a warning.
        assert_eq!(
            phase_line(&Phase::HandoffWrittenWaiting { pending: 0 }, "claude"),
            "📄 handoff written — waiting for the turn to end (up to 15m)"
        );
        assert_eq!(
            phase_line(&Phase::HandoffMissingGrace { left: Duration::from_secs(5) }, "claude"),
            "⏸ handoff file missing — waiting up to 90s for the agent's final reply"
        );
        assert_eq!(
            phase_line(&Phase::BashLinger { pending: 1, left: Duration::from_secs(5) }, "claude"),
            "⏸ background task still running — waiting up to 15m"
        );
        // Identity ignores the countdown, so a ticking window logs once.
        assert_eq!(
            phase_key(&Phase::IdleConfirming { left: Duration::from_secs(19) }),
            phase_key(&Phase::IdleConfirming { left: Duration::from_secs(3) })
        );
        assert_ne!(
            phase_key(&Phase::Subagents { running: 2, done: 0 }),
            phase_key(&Phase::Subagents { running: 1, done: 1 })
        );
        assert!(is_phase_line("📄 handoff file written"));
        assert!(!is_phase_line("✓ step complete (handoff + idle turn)"));
    }
}
