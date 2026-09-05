//! Claude Code adapter: `~/.claude/projects/<cwd-slug>/<sid>.jsonl` records →
//! the normalized model. Shapes are from the corpus census (design §2):
//!
//! * `assistant` — `message.content` has exactly one block; the records of one
//!   API response share a `requestId` and INTERLEAVE with the user
//!   `tool_result` records of parallel tools → turns are grouped by
//!   `requestId`, never by line adjacency.
//! * `user` — either typed prose (string or `text`/`image` blocks) or the
//!   `tool_result` blocks for a prior call; `toolUseResult` rides on the record
//!   and is per-tool structured OR a bare string (1.5%).
//! * `system` (`turn_duration`, `stop_hook_summary`), `attachment`,
//!   `queue-operation`, `ai-title`, `pr-link`, `cost-state` and the
//!   uuid-less sidecars (`last-prompt`, `mode`, `permission-mode`, `atis-latch`,
//!   `bridge-session`, `file-history-*`).
//! * `thinking` blocks persist only a signature → a `thinking` marker.
//!
//! Cost/tokens come from per-record `message.usage` deduped by
//! `(message.id, requestId)` exactly like `otto-usage`; `cost-state` is only a
//! fallback when no usage was seen.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::fold::{BlockRef, Fold, FoldOpts, Folded};
use crate::model::*;
use crate::records::UNPARSEABLE_TYPE;
use crate::util::{
    basename, clip, extract_pseudo_tags, first_line, pr_urls, str_of, string_of,
    structured_patch_to_unified, u64_of,
};

/// Record types that are pure sidecars: known, carry nothing the conversation
/// shows, and are re-emitted redundantly (`ai-title` up to 145× per session).
const SIDECARS: &[&str] = &[
    "last-prompt",
    "mode",
    "permission-mode",
    "atis-latch",
    "bridge-session",
    "file-history-snapshot",
    "file-history-delta",
];

/// Fold a whole Claude transcript (or one `subagents/agent-<id>.jsonl`).
pub fn fold_claude(records: &[Value], opts: FoldOpts<'_>) -> Folded {
    let mut st = ClaudeFolder::new(opts);
    for v in records {
        st.push(v);
    }
    st.into_folded()
}

/// Incremental Claude fold: push records as they appear (live tail), snapshot
/// any time. Claude never changes its mind about earlier records, so `push`
/// never asks for a refold.
#[derive(Clone)]
pub struct ClaudeFolder<'a> {
    st: ClaudeState<'a>,
    count: usize,
}

impl<'a> ClaudeFolder<'a> {
    pub fn new(opts: FoldOpts<'a>) -> Self {
        Self {
            st: ClaudeState {
                f: Fold::new(Provider::Claude, opts),
                req_turn: HashMap::new(),
                usage_seen: HashSet::new(),
                tasks: Vec::new(),
                task_creates: HashMap::new(),
            },
            count: 0,
        }
    }

    pub fn push(&mut self, v: &Value) {
        self.st.record(self.count, v);
        self.count += 1;
    }

    pub fn record_count(&self) -> usize {
        self.count
    }

    /// Replace the subagent tree used at snapshot time (sidecars appear late).
    pub fn set_subagents(&mut self, subagents: Vec<crate::model::SubagentMeta>) {
        self.st.f.opts.subagents = subagents;
    }

    pub fn snapshot(&self) -> Folded {
        self.st.f.snapshot(self.count)
    }

    pub fn into_folded(self) -> Folded {
        self.st.f.finish(self.count)
    }
}

#[derive(Clone)]
struct ClaudeState<'a> {
    f: Fold<'a>,
    /// `requestId` → assistant turn index.
    req_turn: HashMap<String, usize>,
    /// `(message.id, requestId)` keys already counted.
    usage_seen: HashSet<String>,
    /// Running task-list state (`TodoWrite` replaces, `TaskCreate`/`TaskUpdate`
    /// mutate); each task tool call emits a snapshot AFTER the call.
    tasks: Vec<TaskItem>,
    /// `TaskCreate` tool id → (tasks block, subject) so the `toolUseResult.task.id`
    /// arriving later can fill the `ext_id`.
    task_creates: HashMap<String, (BlockRef, String)>,
}

impl ClaudeState<'_> {
    fn record(&mut self, idx: usize, v: &Value) {
        // Identity fields are on most records; take the first non-empty ones.
        if self.f.session_id.is_none() {
            self.f.session_id = string_of(v, "sessionId").or_else(|| string_of(v, "session_id"));
        }
        if self.f.cwd.is_none() {
            self.f.cwd = string_of(v, "cwd");
        }
        self.f.saw_ts(str_of(v, "timestamp"));

        match str_of(v, "type") {
            Some("user") => self.user(idx, v),
            Some("assistant") => self.assistant(idx, v),
            Some("system") => self.system(idx, v),
            Some("attachment") => {
                let a = v.get("attachment").cloned().unwrap_or(Value::Null);
                let kind = str_of(&a, "type").unwrap_or("attachment");
                let body = ["content", "text", "message", "filename", "path"]
                    .iter()
                    .find_map(|k| str_of(&a, k))
                    .map(|s| clip(s, 500));
                let t = self.f.last_turn();
                self.f.note(
                    t,
                    SystemNote {
                        kind: SystemNoteKind::Attachment,
                        title: kind.replace('_', " "),
                        body,
                    },
                    idx,
                );
            }
            Some("queue-operation") => {
                let op = str_of(v, "operation").and_then(QueueOp::parse).unwrap_or(QueueOp::Enqueue);
                let text = str_of(v, "content").unwrap_or("").to_string();
                let trimmed = text.trim_start();
                let injected = trimmed.starts_with("<task-notification>")
                    || trimmed.starts_with("<system-reminder>")
                    || trimmed.starts_with("<system");
                let t = self.f.last_turn();
                self.f.block_or_pending(t, Block::Queued { op, text, injected }, idx);
            }
            Some("ai-title") => {
                if let Some(t) = str_of(v, "aiTitle").filter(|s| !s.trim().is_empty()) {
                    self.f.title = Some(t.to_string());
                }
            }
            Some("pr-link") => {
                let Some(url) = string_of(v, "prUrl") else { return };
                let t = self.f.last_turn();
                let turn_id = t.map(|t| self.f.turns[t].turn.id.clone()).unwrap_or_default();
                let ts = string_of(v, "timestamp");
                if let Some(mut art) = self.f.artifact(ArtifactKind::Pr, None, Some(url), &turn_id, ts) {
                    if let (Some(repo), Some(n)) = (str_of(v, "prRepository"), u64_of(v, "prNumber")) {
                        art.label = format!("{repo}#{n}");
                    }
                    self.f.block_or_pending(t, Block::Artifact { artifact: art }, idx);
                }
            }
            Some("cost-state") => {
                if let Some(c) = v.get("totalCostUSD").and_then(Value::as_f64) {
                    self.f.cost_fallback = Some(c);
                }
            }
            Some(t) if SIDECARS.contains(&t) => {}
            Some(UNPARSEABLE_TYPE) => self.f.unknown("unparseable line", idx),
            Some(other) => self.f.unknown(other, idx),
            None => self.f.unknown("(untyped)", idx),
        }
    }

    // ── user ───────────────────────────────────────────────────────────────

    fn user(&mut self, idx: usize, v: &Value) {
        let msg = v.get("message").cloned().unwrap_or(Value::Null);
        let ts = string_of(v, "timestamp");
        let uuid = string_of(v, "uuid").unwrap_or_else(|| format!("r{idx}"));
        let tur = v.get("toolUseResult");

        let mut prose = String::new();
        let mut images: Vec<(String, String)> = Vec::new(); // (id, media_type)
        let mut touched: Option<usize> = None;

        match msg.get("content") {
            Some(Value::String(s)) => prose.push_str(s),
            Some(Value::Array(blocks)) => {
                for b in blocks {
                    match str_of(b, "type") {
                        Some("text") => {
                            if let Some(t) = str_of(b, "text") {
                                if !prose.is_empty() {
                                    prose.push_str("\n\n");
                                }
                                prose.push_str(t);
                            }
                        }
                        Some("image") => {
                            let src = b.get("source").cloned().unwrap_or(Value::Null);
                            let media = str_of(&src, "media_type").unwrap_or("image/png").to_string();
                            if let Some(data) = str_of(&src, "data") {
                                let id = self.f.image(&media, data);
                                images.push((id, media));
                            }
                        }
                        Some("tool_result") => {
                            if let Some(t) = self.tool_result(idx, b, tur, &ts) {
                                touched = Some(t);
                            }
                        }
                        _ => {
                            // Unknown content block inside a user record.
                            let t = self.f.last_turn();
                            self.f.note(
                                t,
                                SystemNote {
                                    kind: SystemNoteKind::Other,
                                    title: format!("user block: {}", str_of(b, "type").unwrap_or("?")),
                                    body: None,
                                },
                                idx,
                            );
                        }
                    }
                }
            }
            _ => {}
        }

        let (clean, notes) = extract_pseudo_tags(&prose);
        if !clean.is_empty() || !images.is_empty() {
            let t = self.f.new_turn(uuid, Role::User, ts, None, idx);
            if !clean.is_empty() {
                if self.f.first_prompt.is_none() {
                    self.f.first_prompt = Some(clip(&clean, 300));
                }
                self.f.push_block(t, Block::Text { md: clean }, idx);
            }
            for (n, (id, media_type)) in images.into_iter().enumerate() {
                self.f.push_block(
                    t,
                    Block::Image {
                        id: id.clone(),
                        media_type,
                        alt: Some(format!("Image #{}", n + 1)),
                    },
                    idx,
                );
                let path = self.f.opts.images.as_ref().map(|s| s.dir().join(format!("{id}.png")).to_string_lossy().into_owned());
                let turn_id = self.f.turns[t].turn.id.clone();
                let ts = self.f.turns[t].turn.ts.clone();
                self.f.artifact(ArtifactKind::Image, path.or_else(|| Some(format!("img:{id}"))), None, &turn_id, ts);
            }
            for n in notes {
                self.f.note(Some(t), n, idx);
            }
        } else {
            // System-only or tool-result-only record: notes go to the turn the
            // results landed on, else the last turn, else the next one.
            let target = touched.or_else(|| self.f.last_turn());
            for n in notes {
                self.f.note(target, n, idx);
            }
        }
    }

    /// Attach one `tool_result` block (+ the record's `toolUseResult`) to its
    /// call. Returns the owning turn, or `None` for an orphan (which becomes a
    /// notice so it is not lost).
    fn tool_result(&mut self, idx: usize, b: &Value, tur: Option<&Value>, ts: &Option<String>) -> Option<usize> {
        let tool_id = str_of(b, "tool_use_id").unwrap_or("").to_string();
        let is_error = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
        let mut text = String::new();
        let mut image_ids = Vec::new();
        match b.get("content") {
            Some(Value::String(s)) => text.push_str(s),
            Some(Value::Array(parts)) => {
                for p in parts {
                    match str_of(p, "type") {
                        Some("text") => {
                            if let Some(t) = str_of(p, "text") {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(t);
                            }
                        }
                        Some("image") => {
                            let src = p.get("source").cloned().unwrap_or(Value::Null);
                            let media = str_of(&src, "media_type").unwrap_or("image/png");
                            if let Some(data) = str_of(&src, "data") {
                                image_ids.push(self.f.image(media, data));
                            }
                        }
                        Some("tool_reference") => {
                            if let Some(n) = str_of(p, "tool_name") {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(n);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        let mut result = Fold::result_from_text(!is_error, &text, image_ids);
        // Per-tool structured payload — every access tolerant (bare strings occur).
        if let Some(t) = tur.filter(|t| t.is_object()) {
            if result.text.is_none() {
                let mut out = String::new();
                if let Some(s) = str_of(t, "stdout") {
                    out.push_str(s);
                }
                if let Some(e) = str_of(t, "stderr").filter(|e| !e.trim().is_empty()) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(e);
                }
                if !out.is_empty() {
                    let r2 = Fold::result_from_text(result.ok, &out, Vec::new());
                    result.text = r2.text;
                    result.truncated = r2.truncated;
                    result.bytes = r2.bytes;
                }
            }
            let file_path = string_of(t, "filePath").or_else(|| t.get("file").and_then(|f| string_of(f, "filePath")));
            if let Some(fp) = &file_path {
                result.patch = t.get("structuredPatch").and_then(|p| structured_patch_to_unified(p, Some(fp)));
            }
            result.file_path = file_path;
            if let Some(agent_id) = str_of(t, "agentId") {
                self.f.agent_ids.insert(tool_id.clone(), agent_id.to_string());
            }
            // TaskCreate → the provider-side task id arrives with the result.
            if let Some(task_id) = t.get("task").and_then(|task| str_of(task, "id")) {
                if let Some((bref, subject)) = self.task_creates.remove(&tool_id) {
                    for item in self.tasks.iter_mut().filter(|i| i.ext_id.is_none() && i.title == subject) {
                        item.ext_id = Some(task_id.to_string());
                    }
                    if let Some(Block::Tasks { tasks }) = self.f.turns.get_mut(bref.turn).and_then(|ft| ft.turn.blocks.get_mut(bref.block)) {
                        for item in tasks.iter_mut().filter(|i| i.ext_id.is_none() && i.title == subject) {
                            item.ext_id = Some(task_id.to_string());
                        }
                    }
                }
            }
        }
        match self.f.attach_result(&tool_id, result, idx) {
            Some(r) => Some(r.turn),
            None => {
                let t = self.f.last_turn();
                self.f.note(
                    t,
                    SystemNote {
                        kind: SystemNoteKind::Other,
                        title: "Orphan tool result".into(),
                        body: Some(clip(&text, 300)),
                    },
                    idx,
                );
                let _ = ts;
                None
            }
        }
    }

    // ── assistant ──────────────────────────────────────────────────────────

    fn assistant(&mut self, idx: usize, v: &Value) {
        let msg = v.get("message").cloned().unwrap_or(Value::Null);
        let rid = string_of(v, "requestId");
        let model = string_of(&msg, "model");
        if model.is_some() {
            self.f.model = model.clone();
        }
        let ts = string_of(v, "timestamp");
        let t = match rid.as_ref().and_then(|r| self.req_turn.get(r)).copied() {
            Some(t) => {
                self.f.touch(t, idx);
                t
            }
            None => {
                let id = rid.clone().or_else(|| string_of(v, "uuid")).unwrap_or_else(|| format!("r{idx}"));
                let t = self.f.new_turn(id, Role::Assistant, ts.clone(), model.clone(), idx);
                if let Some(r) = rid {
                    self.req_turn.insert(r, t);
                }
                t
            }
        };
        // Usage, counted once per API response.
        if let Some(usage) = msg.get("usage").filter(|u| u.is_object()) {
            let key = format!(
                "{}:{}",
                str_of(&msg, "id").unwrap_or(""),
                str_of(v, "requestId").unwrap_or("")
            );
            let count = if key == ":" { true } else { self.usage_seen.insert(key) };
            if count {
                self.f.usage(
                    model.as_deref().unwrap_or(""),
                    u64_of(usage, "input_tokens").unwrap_or(0),
                    u64_of(usage, "output_tokens").unwrap_or(0),
                    u64_of(usage, "cache_read_input_tokens").unwrap_or(0),
                    u64_of(usage, "cache_creation_input_tokens").unwrap_or(0),
                );
            }
        }
        let blocks: Vec<Value> = match msg.get("content") {
            Some(Value::Array(a)) => a.clone(),
            Some(Value::String(s)) => vec![serde_json::json!({ "type": "text", "text": s })],
            _ => Vec::new(),
        };
        for b in &blocks {
            match str_of(b, "type") {
                Some("text") => {
                    let text = str_of(b, "text").unwrap_or("");
                    if text.trim().is_empty() {
                        continue;
                    }
                    self.f.push_block(t, Block::Text { md: text.to_string() }, idx);
                    let turn_id = self.f.turns[t].turn.id.clone();
                    for url in pr_urls(text) {
                        self.f.artifact(ArtifactKind::Pr, None, Some(url), &turn_id, ts.clone());
                    }
                }
                Some("thinking") | Some("redacted_thinking") => self.f.thinking(t, idx),
                Some("tool_use") => self.tool_use(idx, t, b, &ts),
                Some("image") => {
                    let src = b.get("source").cloned().unwrap_or(Value::Null);
                    let media = str_of(&src, "media_type").unwrap_or("image/png").to_string();
                    if let Some(data) = str_of(&src, "data") {
                        let id = self.f.image(&media, data);
                        self.f.push_block(t, Block::Image { id, media_type: media, alt: None }, idx);
                    }
                }
                other => self.f.note(
                    Some(t),
                    SystemNote {
                        kind: SystemNoteKind::Other,
                        title: format!("assistant block: {}", other.unwrap_or("?")),
                        body: None,
                    },
                    idx,
                ),
            }
        }
    }

    fn tool_use(&mut self, idx: usize, t: usize, b: &Value, ts: &Option<String>) {
        let id = str_of(b, "id").unwrap_or("").to_string();
        let name = str_of(b, "name").unwrap_or("tool").to_string();
        let input = b.get("input").cloned().unwrap_or(Value::Null);
        let kind = tool_kind_for_claude(&name);
        let title = claude_tool_title(&name, &input);
        let r = self.f.push_tool_call(t, id.clone(), name.clone(), kind, title, input.clone(), None, idx);
        let turn_id = self.f.turns[t].turn.id.clone();
        match name.as_str() {
            "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
                if let Some(p) = str_of(&input, "file_path").or_else(|| str_of(&input, "notebook_path")) {
                    let kind = if crate::util::mime_for_path(p).is_some_and(|m| m.starts_with("image/")) {
                        ArtifactKind::Image
                    } else {
                        ArtifactKind::File
                    };
                    self.f.artifact(kind, Some(p.to_string()), None, &turn_id, ts.clone());
                }
            }
            "TodoWrite" => {
                if let Some(todos) = input.get("todos").and_then(Value::as_array) {
                    self.tasks = todos
                        .iter()
                        .map(|td| TaskItem {
                            ext_id: None,
                            title: str_of(td, "content").or_else(|| str_of(td, "activeForm")).unwrap_or("(task)").to_string(),
                            status: str_of(td, "status").and_then(TaskItemStatus::parse).unwrap_or(TaskItemStatus::Pending),
                            active_form: string_of(td, "activeForm"),
                        })
                        .collect();
                }
                self.f.push_block(t, Block::Tasks { tasks: self.tasks.clone() }, idx);
            }
            "TaskCreate" => {
                let subject = str_of(&input, "subject").unwrap_or("(task)").to_string();
                self.tasks.push(TaskItem {
                    ext_id: None,
                    title: subject.clone(),
                    status: TaskItemStatus::Pending,
                    active_form: string_of(&input, "activeForm"),
                });
                let b = self.f.push_block(t, Block::Tasks { tasks: self.tasks.clone() }, idx);
                self.task_creates.insert(id, (BlockRef { turn: t, block: b }, subject));
            }
            "TaskUpdate" => {
                let task_id = str_of(&input, "taskId");
                let status = str_of(&input, "status").and_then(TaskItemStatus::parse);
                for item in self.tasks.iter_mut() {
                    if item.ext_id.as_deref() == task_id && task_id.is_some() {
                        if let Some(s) = status {
                            item.status = s;
                        }
                        if let Some(sub) = str_of(&input, "subject") {
                            item.title = sub.to_string();
                        }
                        if let Some(af) = str_of(&input, "activeForm") {
                            item.active_form = Some(af.to_string());
                        }
                    }
                }
                self.f.push_block(t, Block::Tasks { tasks: self.tasks.clone() }, idx);
            }
            _ => {}
        }
        let _ = r;
    }

    // ── system ─────────────────────────────────────────────────────────────

    fn system(&mut self, idx: usize, v: &Value) {
        match str_of(v, "subtype") {
            Some("turn_duration") => {
                let ms = u64_of(v, "durationMs").unwrap_or(0);
                self.f.add_duration(ms);
                if let Some(t) = self.f.last_turn_with_role(Role::Assistant) {
                    self.f.turns[t].turn.duration_ms = Some(ms);
                    self.f.touch(t, idx);
                }
            }
            Some("stop_hook_summary") => {
                let n = u64_of(v, "hookCount").unwrap_or(0);
                let errors = v.get("hookErrors").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
                let body = (errors > 0).then(|| format!("{errors} hook error(s)"));
                let t = self.f.last_turn();
                self.f.note(
                    t,
                    SystemNote {
                        kind: SystemNoteKind::Hook,
                        title: format!("Stop hook{}", if n == 1 { String::new() } else { format!("s ×{n}") }),
                        body,
                    },
                    idx,
                );
            }
            other => {
                let t = self.f.last_turn();
                let body = ["content", "message", "text"].iter().find_map(|k| str_of(v, k)).map(|s| clip(s, 500));
                self.f.note(
                    t,
                    SystemNote {
                        kind: SystemNoteKind::Other,
                        title: other.unwrap_or("system").replace('_', " "),
                        body,
                    },
                    idx,
                );
            }
        }
    }
}

/// One-line title for a Claude tool call, from its input.
pub fn claude_tool_title(name: &str, input: &Value) -> String {
    let s = |k: &str| str_of(input, k).unwrap_or("");
    match name {
        "Bash" => {
            let d = s("description");
            if !d.is_empty() {
                clip(d, 120)
            } else {
                first_line(s("command"), 120)
            }
        }
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "NotebookRead" => {
            let p = if s("file_path").is_empty() { s("notebook_path") } else { s("file_path") };
            if p.is_empty() {
                name.to_string()
            } else {
                basename(p).to_string()
            }
        }
        "Grep" | "Glob" => {
            let p = s("pattern");
            if p.is_empty() {
                name.to_string()
            } else {
                format!("{name} {}", clip(p, 100))
            }
        }
        "ToolSearch" => format!("ToolSearch {}", clip(s("query"), 100)),
        "Agent" | "Task" => {
            let d = s("description");
            if d.is_empty() {
                clip(s("subagent_type"), 80)
            } else {
                clip(d, 120)
            }
        }
        "Skill" => {
            let c = if s("command").is_empty() { s("skill") } else { s("command") };
            format!("Skill {}", clip(c, 80))
        }
        "WebFetch" => clip(s("url"), 120),
        "WebSearch" => clip(s("query"), 120),
        "TodoWrite" => "Update todos".to_string(),
        "TaskCreate" => clip(s("subject"), 120),
        "TaskUpdate" => format!("Task #{} → {}", s("taskId"), s("status")),
        "AskUserQuestion" => "Question for you".to_string(),
        n if n.starts_with("mcp__") => n.trim_start_matches("mcp__").replace("__", " · "),
        n => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::parse_records;

    fn fold(jsonl: &str) -> Folded {
        fold_claude(&parse_records(jsonl.as_bytes()), FoldOpts::default())
    }

    const SESSION: &str = r#"{"type":"user","uuid":"u1","timestamp":"2026-09-01T10:00:00Z","sessionId":"s1","cwd":"/repo","message":{"role":"user","content":"fix the bug\n<system-reminder>be nice</system-reminder>"}}
{"type":"assistant","uuid":"a1","requestId":"req_1","timestamp":"2026-09-01T10:00:01Z","message":{"id":"msg_1","model":"claude-x","content":[{"type":"thinking","thinking":"","signature":"x"}],"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100,"cache_creation_input_tokens":0}}}
{"type":"assistant","uuid":"a2","requestId":"req_1","timestamp":"2026-09-01T10:00:02Z","message":{"id":"msg_1","model":"claude-x","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls","description":"List"}}],"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100,"cache_creation_input_tokens":0}}}
{"type":"assistant","uuid":"a3","requestId":"req_1","timestamp":"2026-09-01T10:00:02Z","message":{"id":"msg_1","model":"claude-x","content":[{"type":"tool_use","id":"toolu_2","name":"Edit","input":{"file_path":"/repo/a.rs","old_string":"x","new_string":"y"}}],"usage":{"input_tokens":10,"output_tokens":5}}}
{"type":"user","uuid":"u2","timestamp":"2026-09-01T10:00:03Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"a.rs\nb.rs","is_error":false}]},"toolUseResult":{"stdout":"a.rs\nb.rs","stderr":"","interrupted":false}}
{"type":"user","uuid":"u3","timestamp":"2026-09-01T10:00:04Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_2","content":"ok"}]},"toolUseResult":{"filePath":"/repo/a.rs","structuredPatch":[{"oldStart":1,"oldLines":1,"newStart":1,"newLines":1,"lines":["-x","+y"]}]}}
{"type":"assistant","uuid":"a4","requestId":"req_2","timestamp":"2026-09-01T10:00:05Z","message":{"id":"msg_2","model":"claude-x","content":[{"type":"text","text":"Done. See https://github.com/o/r/pull/5"}],"usage":{"input_tokens":1,"output_tokens":1}}}
{"type":"system","subtype":"turn_duration","durationMs":4200,"timestamp":"2026-09-01T10:00:06Z","uuid":"sys1"}
{"type":"ai-title","aiTitle":"Fix the bug","sessionId":"s1"}
{"type":"attachment","uuid":"at1","attachment":{"type":"total_tokens_reminder"}}
{"type":"mode","mode":"normal","sessionId":"s1"}
{"type":"weird-new-thing","x":1}
"#;

    #[test]
    fn groups_by_request_id_and_attaches_results() {
        let f = fold(SESSION);
        assert_eq!(f.session_id.as_deref(), Some("s1"));
        assert_eq!(f.title.as_deref(), Some("Fix the bug"));
        assert_eq!(f.cwd.as_deref(), Some("/repo"));
        assert_eq!(f.model.as_deref(), Some("claude-x"));
        assert_eq!(f.first_prompt.as_deref(), Some("fix the bug"));
        // user, assistant(req_1), assistant(req_2)
        assert_eq!(f.turns.len(), 3);
        let u = &f.turns[0].turn;
        assert_eq!(u.id, "u1");
        assert!(matches!(&u.blocks[0], Block::Text { md } if md == "fix the bug"));
        assert_eq!(u.system[0].kind, SystemNoteKind::SystemReminder);
        let a = &f.turns[1].turn;
        assert_eq!(a.id, "req_1");
        assert!(matches!(a.blocks[0], Block::Thinking { count: 1 }));
        let Block::ToolCall { result: Some(r1), tool, .. } = &a.blocks[1] else { panic!("tool call") };
        assert_eq!(*tool, ToolKind::Shell);
        assert_eq!(r1.text.as_deref(), Some("a.rs\nb.rs"));
        assert!(r1.ok);
        let Block::ToolCall { result: Some(r2), .. } = &a.blocks[2] else { panic!("tool call") };
        assert_eq!(r2.file_path.as_deref(), Some("/repo/a.rs"));
        assert!(r2.patch.as_deref().unwrap().contains("-x\n+y"));
        // duration lands on the last assistant turn; span covers the results.
        assert_eq!(f.turns[2].turn.duration_ms, Some(4200));
        assert_eq!(f.turns[1].last, 5);
        // usage deduped: msg_1 counted once (3 records), msg_2 once.
        assert_eq!(f.stats.input_tokens, Some(10 + 100 + 1));
        assert_eq!(f.stats.output_tokens, Some(6));
        assert_eq!(f.stats.tool_calls, 2);
        assert_eq!(f.stats.thinking_steps, 1);
        assert_eq!(f.stats.turns, 3);
        assert_eq!(f.stats.duration_ms, Some(4200));
        // unknown record → 1, with a notice on the last turn; sidecars silent.
        assert_eq!(f.stats.unknown_records, 1);
        assert!(f.turns[2].turn.blocks.iter().any(|b| matches!(b, Block::Notice { .. })));
        assert!(f.turns[2].turn.system.iter().any(|n| n.kind == SystemNoteKind::Attachment));
        // artifacts: the edited file + the PR url in prose.
        assert_eq!(f.artifacts.len(), 2);
        assert!(f.artifacts.iter().any(|a| a.kind == ArtifactKind::File && a.path.as_deref() == Some("/repo/a.rs")));
        assert!(f.artifacts.iter().any(|a| a.kind == ArtifactKind::Pr && a.label == "o/r#5"));
    }

    #[test]
    fn price_callback_feeds_cost() {
        let price = |_m: &str, i: u64, o: u64, _cr: u64, _cw: u64| (i + o) as f64 * 0.5;
        let recs = parse_records(SESSION.as_bytes());
        let f = fold_claude(&recs, FoldOpts { price: Some(&price), ..Default::default() });
        // msg_1: 10+5, msg_2: 1+1 → 17 * 0.5
        assert_eq!(f.stats.cost_usd, Some(8.5));
    }

    #[test]
    fn bare_string_tool_use_result_and_string_content_are_fine() {
        let jsonl = r#"{"type":"assistant","uuid":"a1","requestId":"r","message":{"model":"m","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/repo/big.txt"}}]}}
{"type":"user","uuid":"u1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"Error: too big","is_error":true}]},"toolUseResult":"Error: too big"}
"#;
        let f = fold(jsonl);
        let Block::ToolCall { result: Some(r), title, .. } = &f.turns[0].turn.blocks[0] else { panic!() };
        assert!(!r.ok);
        assert_eq!(title, "big.txt");
        assert_eq!(f.stats.unknown_records, 0);
    }

    #[test]
    fn images_get_ids_and_tasks_track_state() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
        let jsonl = format!(
            r#"{{"type":"user","uuid":"u1","message":{{"role":"user","content":[{{"type":"text","text":"[Image #1] look"}},{{"type":"image","source":{{"type":"base64","media_type":"image/png","data":"{png}"}}}}]}}}}
{{"type":"assistant","uuid":"a1","requestId":"r1","message":{{"model":"m","content":[{{"type":"tool_use","id":"t1","name":"TaskCreate","input":{{"subject":"Do X"}}}}]}}}}
{{"type":"user","uuid":"u2","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":"Task #1 created"}}]}},"toolUseResult":{{"task":{{"id":"1","subject":"Do X"}}}}}}
{{"type":"assistant","uuid":"a2","requestId":"r2","message":{{"model":"m","content":[{{"type":"tool_use","id":"t2","name":"TaskUpdate","input":{{"taskId":"1","status":"completed"}}}}]}}}}
{{"type":"queue-operation","operation":"enqueue","content":"next please"}}
"#
        );
        let f = fold(&jsonl);
        let u = &f.turns[0].turn;
        assert!(matches!(&u.blocks[1], Block::Image { alt: Some(a), .. } if a == "Image #1"));
        let Block::Tasks { tasks } = &f.turns[1].turn.blocks[1] else { panic!("tasks block") };
        assert_eq!(tasks[0].ext_id.as_deref(), Some("1"), "ext_id filled from the result");
        let Block::Tasks { tasks } = &f.turns[2].turn.blocks[1] else { panic!("tasks block") };
        assert_eq!(tasks[0].status, TaskItemStatus::Completed);
        assert!(f.turns[2].turn.blocks.iter().any(|b| matches!(b, Block::Queued { op: QueueOp::Enqueue, injected: false, .. })));
        assert!(f.artifacts.iter().any(|a| a.kind == ArtifactKind::Image));
    }

    #[test]
    fn subagent_blocks_attach_to_agent_calls() {
        let jsonl = r#"{"type":"assistant","uuid":"a1","requestId":"r1","message":{"model":"m","content":[{"type":"tool_use","id":"toolu_A","name":"Agent","input":{"description":"Explore","subagent_type":"Explore"}}]}}
{"type":"user","uuid":"u1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_A","content":"done"}]},"toolUseResult":{"agentId":"abc123"}}
"#;
        let metas = vec![SubagentMeta {
            agent_id: "abc123".into(),
            parent_agent_id: None,
            depth: 1,
            agent_type: "Explore".into(),
            description: "Explore".into(),
            model: None,
            tool_use_id: Some("toolu_A".into()),
        }];
        let recs = parse_records(jsonl.as_bytes());
        let f = fold_claude(&recs, FoldOpts { subagents: metas.clone(), ..Default::default() });
        let blocks = &f.turns[0].turn.blocks;
        assert!(matches!(&blocks[1], Block::Subagent { agent_id, status: Some(SubagentStatus::Done), .. } if agent_id == "abc123"));
        // Without a sidecar the result's agentId still yields a block.
        let f2 = fold_claude(&recs, FoldOpts::default());
        assert!(matches!(&f2.turns[0].turn.blocks[1], Block::Subagent { agent_id, .. } if agent_id == "abc123"));
    }
}
