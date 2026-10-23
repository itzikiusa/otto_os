//! The "is the agent's turn still open?" folds over Claude transcripts and
//! Codex rollouts (architecture-6-02) — moved here from
//! `otto-agent-run::turn_oracle` so the turn oracle (workflow steps, review
//! engine) and the session manager's idle sweep read ONE rule. The oracle
//! keeps its clocks and verdicts; the sweep asks [`tail_open`].
//!
//! Pure folds over transcript lines plus the byte-offset tails that feed them
//! incrementally ([`ClaudeTail`], [`CodexTail`]; blocking IO — call off the
//! runtime).
//!
//! Notification rule (one, shared): a `<task-notification>` is parsed
//! structurally (`<task-id>` + `<status>`, [`find_task_notifications`]); it
//! clears the task and re-opens the parent's turn — except a `queue-operation`
//! `remove` (never delivered) and an `attachment` line (claude staples a late
//! notification onto the user's NEXT prompt instead of waking the parent, so
//! an attachment tail is the normal end state of a finished session).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
    /// The latest typed parent user prompt (sidechains excluded) — what the
    /// submit-confirm loop matches its prompt slice against.
    pub last_user_text: Option<String>,
    /// Number of parent user prompts, used to confirm a newly submitted turn.
    pub user_turns: usize,
    /// Completed assistant turns before the latest parent prompt (not its reply).
    pub completed_before_last_user: usize,
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

impl CodexScan {
    /// Is the latest turn still running? `Some(true)` = a `task_started` with
    /// no `task_complete` / `turn_aborted` of the SAME turn yet (a stale
    /// turn's completion never closes the live one); `None` = no turn seen.
    pub fn tail_open(&self) -> Option<bool> {
        if self.turn_complete || (self.latest_turn.is_none() && self.aborted_at.is_some()) {
            Some(false)
        } else {
            self.latest_turn.as_ref().map(|_| true)
        }
    }
}

/// The idle sweep's question over a transcript text window: is `provider`'s
/// turn open at the window's end? `None` for a provider without a rule, or a
/// window that holds no parent message (claude) / no turn (codex). Partial
/// lines (the window edge, a mid-append) fold as nothing.
pub fn tail_open(provider: &str, text: &str) -> Option<bool> {
    match provider {
        "claude" => {
            let mut st = ClaudeScanState::default();
            for line in text.lines() {
                st.feed_line(line);
            }
            st.tail_open()
        }
        "codex" => {
            let mut scan = CodexScan::default();
            for line in text.lines() {
                feed_codex_line(&mut scan, line, None);
            }
            scan.tail_open()
        }
        _ => None,
    }
}

/// Walk a claude session JSONL once: turn count/text, the tail state, the
/// pending/notified task sets and the api-error fail-fast. One pass, no I/O.
pub fn scan_claude(jsonl: &str) -> ClaudeScan {
    let mut st = ClaudeScanState::default();
    for line in jsonl.lines() {
        st.feed_line(line);
    }
    st.view()
}

/// [`scan_claude`] as a left fold over lines, so a poller can feed only the
/// lines appended since its last tick ([`ClaudeTail`]) and still get exactly
/// the whole-file answer. Every field is either append-only or "state after
/// the last line", which is what makes the fold exact.
#[derive(Debug, Clone, Default)]
pub struct ClaudeScanState {
    scan: ClaudeScan,
    /// 0-based index of the next line (empty lines count, like `lines()`).
    line: usize,
    // Tail state, folded forward: `tail_end_turn` is "the last message line was
    // an assistant end_turn and nothing since wakes the parent"; the queue flags
    // record whether the ONLY thing since was a queue-operation line.
    tail_end_turn: bool,
    post_end_turn_queue_only: bool,
    saw_queue_after_end_turn: bool,
    /// A parent message line was folded (`tail_open` has something to say).
    saw_message: bool,
}

impl ClaudeScanState {
    /// The scan as of the lines fed so far.
    pub fn view(&self) -> ClaudeScan {
        let mut s = self.scan.clone();
        s.tail_is_assistant_end_turn = self.tail_end_turn;
        s.tail_evidence_is_queue_only =
            self.post_end_turn_queue_only && self.saw_queue_after_end_turn;
        s
    }

    /// Is the parent mid-turn as of the lines fed so far? `Some(true)` = the
    /// last message line is not an assistant `end_turn`, or a delivered
    /// notification since then will wake it; `None` = no parent message line
    /// yet (a fresh or unparseable window says nothing about the turn).
    pub fn tail_open(&self) -> Option<bool> {
        self.saw_message.then_some(!self.tail_end_turn)
    }

    /// Fold one line (without its `\n`).
    pub fn feed_line(&mut self, line: &str) {
        let i = self.line;
        self.line += 1;
        let s = &mut self.scan;
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        // A partially-written trailing line (and any metadata shape we don't
        // parse) contributes nothing — exactly what `completed_turn_count` does.
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        // `transcript_api_error` semantics: the FIRST api-error line wins, and
        // Parent prompt boundaries are counted after the sidechain filter.
        if s.api_error.is_none() {
            s.api_error = api_error_text(&v);
        }
        // Legacy in-file sub-agent lines: never a parent turn, never a task.
        if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
            return;
        }
        if let Some(t) = user_prompt_text(&v) {
            s.last_user_text = Some(t);
            s.user_turns += 1;
            s.completed_before_last_user = s.completed_turns;
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
            self.saw_message = true;
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
                    self.tail_end_turn = true;
                    self.post_end_turn_queue_only = true;
                    self.saw_queue_after_end_turn = false;
                }
                None => {
                    self.tail_end_turn = false;
                    self.post_end_turn_queue_only = false;
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
                push_pending(
                    &mut s.pending,
                    id,
                    TaskKind::Agent,
                    "resumed sub-agent".into(),
                    i,
                );
            }
            // `TaskStop` — the harness kills the task and NEVER sends a
            // `<task-notification>` for it (a `docker info` that timed out into
            // the background and was stopped 3s later held a step for the full
            // linger cap). The stop result is the completion signal.
            if let Some(id) = task_stopped_id(tur) {
                s.pending.retain(|p| p.id != id);
                s.notified.insert(id.clone(), "stopped".to_string());
                if let Some(t) = ts {
                    s.notified_at.insert(id, t);
                }
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
            // An `attachment` notification is stapled onto the user's NEXT
            // prompt, not a wake-up: it is the normal tail of a finished
            // session that ran sub-agents (24 of the 40 most recent
            // transcripts on a reviewer's machine), so it clears the task but
            // leaves the turn closed.
            let removed =
                is_queue_op && v.get("operation").and_then(|o| o.as_str()) == Some("remove");
            if !removed && kind != "attachment" {
                self.tail_end_turn = false;
                if !is_queue_op {
                    // A real wake-up line: the tail is no longer queue-only.
                    self.post_end_turn_queue_only = false;
                }
            }
        }
        if is_queue_op {
            self.saw_queue_after_end_turn = true;
        }
    }
}

/// `otto_orchestrator::claude_pty::transcript_api_error` for ONE parsed line
/// (the scan parses each line once and shares it).
fn api_error_text(v: &serde_json::Value) -> Option<String> {
    if v.get("isApiErrorMessage").and_then(|b| b.as_bool()) != Some(true) {
        return None;
    }
    let msg = v.get("message")?;
    if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
        return None;
    }
    for b in msg.get("content")?.as_array()? {
        if b.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                if !t.trim().is_empty() {
                    return Some(t.trim().to_string());
                }
            }
        }
    }
    None
}

/// `otto_orchestrator::claude_pty::last_user_text` for ONE parsed line: the
/// typed text of a `role:"user"` message (tool results carry no text blocks),
/// `None` when it has none — or when claude flagged the line `isMeta` (an
/// injected skill body, a local-command caveat) or `isCompactSummary` (the
/// `/compact` summary): those are not prompts the user typed
/// (integrations-6-02; same filter as `otto_sessions::titles`).
pub fn user_prompt_text(v: &serde_json::Value) -> Option<String> {
    if is_meta_line(v) {
        return None;
    }
    let msg = v.get("message")?;
    if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
        return None;
    }
    let text = match msg.get("content")? {
        serde_json::Value::String(s) => s.trim().to_string(),
        serde_json::Value::Array(blocks) => {
            let mut t = String::new();
            for block in blocks {
                if block.get("type").and_then(|k| k.as_str()) != Some("text") {
                    continue;
                }
                if let Some(s) = block.get("text").and_then(|s| s.as_str()) {
                    if !s.is_empty() {
                        if !t.is_empty() {
                            t.push('\n');
                        }
                        t.push_str(s);
                    }
                }
            }
            t.trim().to_string()
        }
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

/// Incremental reader of one claude session JSONL for pollers: each
/// [`poll`](Self::poll) reads only the bytes appended since the last one,
/// folds the complete lines into a [`ClaudeScanState`] and answers exactly
/// what `scan_claude(&whole_file)` would. A 68 MB transcript used to be read
/// (9 ms) and parsed (63 ms) whole once a second per running agent; now the
/// first poll reads it once and every later poll costs O(new bytes).
///
/// A trailing line without its `\n` yet is evaluated on a scratch copy of the
/// state (whole-file `scan_claude` sees it too) and re-fed once complete. A
/// file that shrank or was replaced (different inode) restarts from byte 0.
/// Blocking IO — call off the runtime.
#[derive(Debug, Clone)]
pub struct ClaudeTail {
    lines: LineTail,
    state: ClaudeScanState,
}

impl ClaudeTail {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            lines: LineTail::new(path.into()),
            state: ClaudeScanState::default(),
        }
    }

    /// Scan only what lies past byte `offset` — `completed_turn_text` of the
    /// tail a reused session appended after a brief. A file that shrinks below
    /// `offset` (or is replaced) is re-read from `offset`, never from 0: an
    /// older turn must not be mistaken for this one.
    pub fn from_offset(path: impl Into<PathBuf>, offset: u64) -> Self {
        Self {
            lines: LineTail::from(path.into(), offset),
            state: ClaudeScanState::default(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.lines.path
    }

    /// Bytes consumed so far (complete lines + the carried partial line).
    pub fn offset(&self) -> u64 {
        self.lines.offset
    }

    /// Read what was appended and return the up-to-date scan. `None` when the
    /// file does not exist / cannot be opened (the whole-file callers got no
    /// scan in that case either).
    pub fn poll(&mut self) -> Option<ClaudeScan> {
        self.lines.read(&mut self.state)?;
        Some(match self.lines.partial() {
            None => self.state.view(),
            Some(p) => {
                let mut scratch = self.state.clone();
                scratch.feed_line(&p);
                scratch.view()
            }
        })
    }
}

/// Claude's injected (not typed) user lines: `isMeta` / `isCompactSummary`.
pub fn is_meta_line(v: &serde_json::Value) -> bool {
    ["isMeta", "isCompactSummary"]
        .iter()
        .any(|k| v.get(*k).and_then(|b| b.as_bool()) == Some(true))
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

/// The id a `TaskStop` result stopped: `{"message":"Successfully stopped task:
/// <id> (...)","task_id":"<id>","task_type":"local_bash"|"local_agent",...}`.
/// Matched on BOTH fields — a bare `task_id` also rides on other tool results
/// (`TaskOutput`), and those are not completions.
fn task_stopped_id(tur: &serde_json::Value) -> Option<String> {
    let msg = tur.get("message").and_then(|m| m.as_str())?;
    if !msg.starts_with("Successfully stopped task") {
        return None;
    }
    task_id(tur.get("task_id"))
}

/// An opaque harness id: `[A-Za-z0-9_-]{1,64}`, never logged beyond the id.
fn task_id(v: Option<&serde_json::Value>) -> Option<String> {
    let s = v?.as_str()?.trim();
    valid_id(s).then(|| s.to_string())
}

/// An opaque harness id: `[A-Za-z0-9_-]{1,64}`.
pub fn valid_id(s: &str) -> bool {
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
    pending.push(PendingTask {
        id,
        kind,
        description,
        since_line,
    });
}

/// `s` trimmed and cut to `max` chars.
pub fn truncate_chars(s: &str, max: usize) -> String {
    otto_core::text::prefix_chars(s.trim(), max).to_string()
}

/// An RFC 3339 transcript timestamp.
pub fn parse_ts(s: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| SystemTime::from(d.with_timezone(&chrono::Utc)))
}

/// Every `<task-notification>` in one raw transcript line, as `(id, status)`.
/// Plain scanning (no regex dep): find the tag, then this notification's
/// `<task-id>`/`<status>` before the next one starts.
pub fn find_task_notifications(line: &str) -> Vec<(String, String)> {
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
        feed_codex_line(&mut s, line, Some(after_ordinal));
    }
    s
}

/// [`scan_codex`] for one line — the scan is a left fold, so [`CodexTail`]
/// feeds only appended lines and gets the whole-file answer.
/// `after_ordinal: None` folds every line (the sweep's tail window has no
/// submit baseline).
pub fn feed_codex_line(s: &mut CodexScan, line: &str, after_ordinal: Option<u64>) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    let ordinal = v.get("ordinal").and_then(|o| o.as_u64()).unwrap_or(0);
    if after_ordinal.is_some_and(|after| ordinal <= after) {
        return;
    }
    if v.get("type").and_then(|t| t.as_str()) != Some("event_msg") {
        return;
    }
    s.last_event_ordinal = s.last_event_ordinal.max(ordinal);
    let Some(payload) = v.get("payload") else {
        return;
    };
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

/// A left fold over transcript lines that [`LineTail`] drives.
trait LineSink {
    fn reset(&mut self);
    fn feed(&mut self, line: &str);
}

impl LineSink for ClaudeScanState {
    fn reset(&mut self) {
        *self = Self::default();
    }
    fn feed(&mut self, line: &str) {
        self.feed_line(line);
    }
}

/// `CodexScan` + its submit baseline.
#[derive(Debug, Clone, Default)]
struct CodexSink {
    scan: CodexScan,
    after_ordinal: u64,
}

impl LineSink for CodexSink {
    fn reset(&mut self) {
        self.scan = CodexScan::default();
    }
    fn feed(&mut self, line: &str) {
        feed_codex_line(&mut self.scan, line, Some(self.after_ordinal));
    }
}

/// Byte-offset line reader shared by [`ClaudeTail`] / [`CodexTail`]: reads
/// only what was appended since the last call, hands out complete lines and
/// carries the partial one. A file that shrank or was replaced (new inode)
/// reports `restarted` and is re-read from byte 0. Blocking IO.
#[derive(Debug, Clone)]
struct LineTail {
    path: PathBuf,
    /// Where reading starts (and restarts): 0, or a caller's baseline.
    base: u64,
    offset: u64,
    partial: Vec<u8>,
    ino: Option<u64>,
}

impl LineTail {
    fn new(path: PathBuf) -> Self {
        Self::from(path, 0)
    }

    fn from(path: PathBuf, base: u64) -> Self {
        Self {
            path,
            base,
            offset: base,
            partial: Vec::new(),
            ino: None,
        }
    }

    /// Feed each newly completed line to `sink` (reset first when the file
    /// restarted). `None` when the file cannot be opened.
    fn read<S: LineSink>(&mut self, sink: &mut S) -> Option<()> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(&self.path).ok()?;
        let meta = f.metadata().ok()?;
        let len = meta.len();
        #[cfg(unix)]
        let ino = {
            use std::os::unix::fs::MetadataExt;
            Some(meta.ino())
        };
        #[cfg(not(unix))]
        let ino: Option<u64> = None;
        let restarted = len < self.offset || (self.ino.is_some() && self.ino != ino);
        self.ino = ino;
        if restarted {
            self.offset = self.base;
            self.partial.clear();
            sink.reset();
        }
        if len > self.offset {
            let mut fresh = Vec::with_capacity((len - self.offset) as usize);
            let ok = f.seek(SeekFrom::Start(self.offset)).is_ok()
                && f.take(len - self.offset).read_to_end(&mut fresh).is_ok();
            if ok {
                self.offset += fresh.len() as u64;
                let mut buf = std::mem::take(&mut self.partial);
                buf.extend_from_slice(&fresh);
                match buf.iter().rposition(|b| *b == b'\n') {
                    Some(nl) => {
                        for l in buf[..nl].split(|b| *b == b'\n') {
                            sink.feed(&String::from_utf8_lossy(l));
                        }
                        self.partial = buf[nl + 1..].to_vec();
                    }
                    None => self.partial = buf,
                }
            }
        }
        Some(())
    }

    /// The trailing line still being written (whole-file scans see it too).
    fn partial(&self) -> Option<std::borrow::Cow<'_, str>> {
        (!self.partial.is_empty()).then(|| String::from_utf8_lossy(&self.partial))
    }
}

/// Incremental [`scan_codex`] over one rollout (see [`ClaudeTail`]).
#[derive(Debug, Clone)]
pub struct CodexTail {
    lines: LineTail,
    sink: CodexSink,
}

impl CodexTail {
    pub fn new(path: impl Into<PathBuf>, after_ordinal: u64) -> Self {
        Self {
            lines: LineTail::new(path.into()),
            sink: CodexSink {
                scan: CodexScan::default(),
                after_ordinal,
            },
        }
    }

    pub fn path(&self) -> &Path {
        &self.lines.path
    }

    pub fn after_ordinal(&self) -> u64 {
        self.sink.after_ordinal
    }

    /// `scan_codex(&whole_file, after_ordinal)`, reading only new bytes.
    pub fn poll(&mut self) -> Option<CodexScan> {
        self.lines.read(&mut self.sink)?;
        Some(match self.lines.partial() {
            None => self.sink.scan.clone(),
            Some(p) => {
                let mut s = self.sink.scan.clone();
                feed_codex_line(&mut s, &p, Some(self.sink.after_ordinal));
                s
            }
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = r#"{"type":"user","message":{"role":"user","content":"go"}}"#;
    const END_TURN: &str = r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"done"}]}}"#;
    const NOTE: &str =
        "<task-notification><task-id>a1</task-id><status>completed</status></task-notification>";

    fn after_end_turn(line: &str) -> String {
        format!("{USER}\n{END_TURN}\n{line}\n")
    }

    /// The one notification rule the oracle and the idle sweep share: a
    /// delivered structural notification re-opens the turn; a `remove`d queue
    /// entry, an `attachment` staple and a tag-less mention do not. Each case
    /// also lands in `scan_claude`'s tail flag, so the two consumers agree.
    #[test]
    fn notification_rule_is_pinned() {
        let delivered =
            format!(r#"{{"type":"user","message":{{"role":"user","content":"{NOTE}"}}}}"#);
        let queued =
            format!(r#"{{"type":"queue-operation","operation":"enqueue","content":"{NOTE}"}}"#);
        let removed =
            format!(r#"{{"type":"queue-operation","operation":"remove","content":"{NOTE}"}}"#);
        let attached = format!(r#"{{"type":"attachment","content":{{"text":"{NOTE}"}}}}"#);
        // A non-message line that merely mentions the tag (no `<task-id>`):
        // the sweep's old raw-substring rule re-opened the turn on it.
        let tagless =
            r#"{"type":"system","content":"<task-notification>id=1</task-notification>"}"#;
        for (line, open) in [
            (delivered.as_str(), true),
            (queued.as_str(), true),
            (removed.as_str(), false),
            (attached.as_str(), false),
            (tagless, false),
        ] {
            let text = after_end_turn(line);
            assert_eq!(tail_open("claude", &text), Some(open), "{line}");
            assert_eq!(
                scan_claude(&text).tail_is_assistant_end_turn,
                !open,
                "{line}"
            );
        }
        // Every structural form clears the task, even when it doesn't wake.
        assert!(scan_claude(&after_end_turn(&attached))
            .notified
            .contains_key("a1"));
    }

    #[test]
    fn claude_tail_open_needs_a_parent_message() {
        assert_eq!(tail_open("claude", ""), None);
        assert_eq!(tail_open("claude", "{\"type\":\"summary\"}\n"), None);
        let side = r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","stop_reason":"tool_use","content":[]}}"#;
        assert_eq!(tail_open("claude", &format!("{side}\n")), None);
        assert_eq!(tail_open("claude", &after_end_turn(side)), Some(false));
        assert_eq!(tail_open("claude", &format!("{USER}\n")), Some(true));
        assert_eq!(tail_open("agy", &format!("{USER}\n")), None);
    }

    #[test]
    fn codex_tail_open_follows_the_latest_turn_without_a_baseline() {
        // No `ordinal` at all: the sweep's window has no submit baseline.
        let ev = |kind: &str, t: &str| {
            format!(r#"{{"type":"event_msg","payload":{{"type":"{kind}","turn_id":"{t}"}}}}"#)
        };
        let open = |lines: &[String]| tail_open("codex", &(lines.join("\n") + "\n"));
        assert_eq!(open(&[]), None);
        assert_eq!(open(&[ev("task_started", "t1")]), Some(true));
        assert_eq!(
            open(&[ev("task_started", "t1"), ev("task_complete", "t1")]),
            Some(false)
        );
        assert_eq!(
            open(&[ev("task_started", "t1"), ev("turn_aborted", "t1")]),
            Some(false)
        );
        assert_eq!(
            open(&[
                ev("task_started", "t1"),
                ev("task_started", "t2"),
                ev("task_complete", "t1")
            ]),
            Some(true),
            "a stale turn's completion does not close the live one"
        );
        // The oracle's baseline still filters (ordinal-less lines are 0).
        assert_eq!(scan_codex(&ev("task_started", "t1"), 0).latest_turn, None);
    }

    /// integrations-6-02: an injected `isMeta` skill body (or a compact
    /// summary) after the typed prompt is not a new prompt — the submit
    /// confirm loop would otherwise match its slice against the skill text.
    #[test]
    fn meta_user_lines_are_not_prompts() {
        let skill = r#"{"type":"user","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"Base directory for this skill: /x"}]}}"#;
        let compact = r#"{"type":"user","isCompactSummary":true,"message":{"role":"user","content":"This session is being continued"}}"#;
        let scan = scan_claude(&format!("{USER}\n{skill}\n{compact}\n"));
        assert_eq!(scan.last_user_text.as_deref(), Some("go"));
        assert_eq!(scan.user_turns, 1);
    }
}
