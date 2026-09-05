//! Codex adapter: `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<sid>.jsonl` →
//! the normalized model. Codex has TWO eras (design §2), decided PER FILE:
//!
//! * **new** (CLI ≥ 0.147): `event_msg/item_completed` items
//!   (`UserMessage|AgentMessage|Reasoning|CommandExecution|FileChange|
//!   McpToolCall|ContextCompaction|Extension|ImageView`) carry everything the
//!   conversation shows; `response_item` records are the raw API log and are
//!   NOT rendered (they would double-count every message and call).
//! * **old**: no items — the conversation is read from
//!   `event_msg agent_message/user_message/patch_apply_end/mcp_tool_call_end/
//!   web_search_end` plus `response_item function_call/_output` and
//!   `custom_tool_call/_output`. `response_item message` is used only when the
//!   file has no `event_msg` counterpart for that role (they duplicate).
//!
//! Reasoning is never recoverable (`summary: []`, `summary_text: []` in 100% of
//! the corpus) → counted in `stats.reasoning_steps`, not rendered. Tokens come
//! from the LAST cumulative `token_count.info.total_token_usage` (or
//! `token_usage_record.thread_token_usage`), normalized like `otto-usage`
//! (`input` excludes `cached_input_tokens`).

use std::collections::HashMap;

use serde_json::Value;

use crate::fold::{BlockRef, Fold, FoldOpts, Folded};
use crate::model::*;
use crate::records::UNPARSEABLE_TYPE;
use crate::util::{basename, clip, first_line, pr_urls, str_of, string_of, u64_of};

/// Fold a whole Codex rollout. The per-file decisions (era, which duplicated
/// stream to trust) are pre-scanned so no record is ever rendered under the
/// wrong era.
pub fn fold_codex(records: &[Value], opts: FoldOpts<'_>) -> Folded {
    let mut st = CodexFolder::new(opts);
    for v in records {
        st.prescan(v);
    }
    for v in records {
        st.push(v);
    }
    st.into_folded()
}

/// Incremental Codex fold for live tails. The era / duplicate-stream decisions
/// are per FILE, so a record that flips one after earlier records were already
/// rendered (a rollout whose first `item_completed` arrives after we started)
/// makes `push` return `true`: the caller must refold from record 0.
#[derive(Clone)]
pub struct CodexFolder<'a> {
    st: CodexState<'a>,
    count: usize,
}

impl<'a> CodexFolder<'a> {
    pub fn new(opts: FoldOpts<'a>) -> Self {
        Self {
            st: CodexState {
                f: Fold::new(Provider::Codex, opts),
                new_era: false,
                has_agent_msg_events: false,
                has_user_msg_events: false,
                turns_by_id: HashMap::new(),
                cur_asst: None,
                last_tokens: None,
                pending_patch_calls: Vec::new(),
                cli_version: None,
            },
            count: 0,
        }
    }

    /// Learn the per-file flags from a record WITHOUT rendering it (whole-file
    /// folds call this over every record first).
    pub fn prescan(&mut self, v: &Value) {
        self.flip(v);
    }

    /// Set the flags a record implies; `true` when one flipped.
    fn flip(&mut self, v: &Value) -> bool {
        if str_of(v, "type") != Some("event_msg") {
            return false;
        }
        let flag = match v.get("payload").and_then(|p| str_of(p, "type")) {
            Some("item_completed") => &mut self.st.new_era,
            Some("agent_message") => &mut self.st.has_agent_msg_events,
            Some("user_message") => &mut self.st.has_user_msg_events,
            _ => return false,
        };
        let flipped = !*flag;
        *flag = true;
        flipped
    }

    /// Push one record. Returns `true` when earlier records were rendered under
    /// a now-wrong per-file decision — refold everything from record 0.
    pub fn push(&mut self, v: &Value) -> bool {
        let refold = self.flip(v) && self.count > 0;
        self.st.record(self.count, v);
        self.count += 1;
        refold
    }

    pub fn record_count(&self) -> usize {
        self.count
    }

    pub fn set_subagents(&mut self, subagents: Vec<crate::model::SubagentMeta>) {
        self.st.f.opts.subagents = subagents;
    }

    fn with_tokens(mut st: CodexState<'a>) -> CodexState<'a> {
        if let Some((model, input, output, cached)) = st.last_tokens.take() {
            st.f.usage(&model, input.saturating_sub(cached), output, cached, 0);
        }
        st
    }

    pub fn snapshot(&self) -> Folded {
        Self::with_tokens(self.st.clone()).f.finish(self.count)
    }

    pub fn into_folded(self) -> Folded {
        Self::with_tokens(self.st).f.finish(self.count)
    }
}

#[derive(Clone)]
struct CodexState<'a> {
    f: Fold<'a>,
    new_era: bool,
    has_agent_msg_events: bool,
    has_user_msg_events: bool,
    /// New era: `<turn_id>:u` / `<turn_id>:a` → turn index.
    turns_by_id: HashMap<String, usize>,
    /// Old era: the assistant turn currently being appended to.
    cur_asst: Option<usize>,
    /// Last cumulative snapshot: (model, input, output, cached).
    last_tokens: Option<(String, u64, u64, u64)>,
    /// Old era: `apply_patch` function calls awaiting a `patch_apply_end`.
    pending_patch_calls: Vec<BlockRef>,
    cli_version: Option<String>,
}

impl CodexState<'_> {
    fn record(&mut self, idx: usize, v: &Value) {
        self.f.saw_ts(str_of(v, "timestamp"));
        let payload = v.get("payload").cloned().unwrap_or(Value::Null);
        match str_of(v, "type") {
            Some("session_meta") => {
                self.f.session_id = string_of(&payload, "session_id")
                    .or_else(|| string_of(&payload, "id"))
                    .or_else(|| string_of(v, "id"));
                if self.f.cwd.is_none() {
                    self.f.cwd = string_of(&payload, "cwd");
                }
                if let Some(m) = model_of(&payload) {
                    self.f.model = Some(m);
                }
                self.cli_version = string_of(&payload, "cli_version");
            }
            Some("turn_context") => {
                if self.f.cwd.is_none() {
                    self.f.cwd = string_of(&payload, "cwd");
                }
                if let Some(m) = model_of(&payload) {
                    self.f.model = Some(m);
                }
            }
            Some("world_state") => {}
            Some("compacted") => {
                let t = self.f.last_turn();
                self.f.note(
                    t,
                    SystemNote {
                        kind: SystemNoteKind::Compaction,
                        title: "Context compacted".into(),
                        body: None,
                    },
                    idx,
                );
            }
            Some("token_usage_record") => {
                let u = payload
                    .get("thread_token_usage")
                    .or_else(|| payload.get("usage"))
                    .cloned()
                    .unwrap_or(Value::Null);
                self.tokens(&u);
            }
            Some("event_msg") => self.event(idx, v, &payload),
            Some("response_item") => self.response_item(idx, v, &payload),
            Some(UNPARSEABLE_TYPE) => self.f.unknown("unparseable line", idx),
            Some(other) => self.f.unknown(other, idx),
            None => self.f.unknown("(untyped)", idx),
        }
    }

    fn tokens(&mut self, total: &Value) {
        if !total.is_object() {
            return;
        }
        let model = self.f.model.clone().unwrap_or_else(|| "codex".into());
        self.last_tokens = Some((
            model,
            u64_of(total, "input_tokens").unwrap_or(0),
            u64_of(total, "output_tokens").unwrap_or(0),
            u64_of(total, "cached_input_tokens").unwrap_or(0),
        ));
    }

    // ── turn helpers ───────────────────────────────────────────────────────

    /// New era: the turn for `<turn_id>:<suffix>`, created on first use.
    fn turn_for(&mut self, turn_id: &str, role: Role, ts: Option<String>, idx: usize) -> usize {
        let key = format!("{turn_id}:{}", if role == Role::User { "u" } else { "a" });
        if let Some(&t) = self.turns_by_id.get(&key) {
            self.f.touch(t, idx);
            return t;
        }
        let model = (role == Role::Assistant).then(|| self.f.model.clone()).flatten();
        let t = self.f.new_turn(key.clone(), role, ts, model, idx);
        self.turns_by_id.insert(key, t);
        t
    }

    /// Old era: the current assistant turn, created on first use after a user
    /// message.
    fn asst_turn(&mut self, ts: Option<String>, idx: usize) -> usize {
        if let Some(t) = self.cur_asst {
            self.f.touch(t, idx);
            return t;
        }
        let model = self.f.model.clone();
        let t = self.f.new_turn(format!("r{idx}"), Role::Assistant, ts, model, idx);
        self.cur_asst = Some(t);
        t
    }

    fn user_turn_old(&mut self, text: &str, ts: Option<String>, idx: usize) {
        let t = self.f.new_turn(format!("r{idx}"), Role::User, ts, None, idx);
        self.cur_asst = None;
        if self.f.first_prompt.is_none() {
            self.f.first_prompt = Some(clip(text, 300));
        }
        self.f.push_block(t, Block::Text { md: text.to_string() }, idx);
    }

    fn text_block(&mut self, t: usize, text: &str, ts: &Option<String>, idx: usize) {
        if text.trim().is_empty() {
            return;
        }
        self.f.push_block(t, Block::Text { md: text.to_string() }, idx);
        let turn_id = self.f.turns[t].turn.id.clone();
        for url in pr_urls(text) {
            self.f.artifact(ArtifactKind::Pr, None, Some(url), &turn_id, ts.clone());
        }
    }

    // ── event_msg ──────────────────────────────────────────────────────────

    fn event(&mut self, idx: usize, v: &Value, p: &Value) {
        let ts = string_of(v, "timestamp");
        match str_of(p, "type") {
            Some("item_completed") => self.item(idx, v, p),
            Some("token_count") => {
                let total = p.get("info").and_then(|i| i.get("total_token_usage")).cloned().unwrap_or(Value::Null);
                self.tokens(&total);
            }
            Some("task_started") => {
                if !self.new_era {
                    // A fresh turn begins: whatever assistant turn was open is done.
                    self.cur_asst = None;
                }
            }
            Some("task_complete") | Some("turn_aborted") => {
                let ms = u64_of(p, "duration_ms");
                let t = if self.new_era {
                    str_of(p, "turn_id").and_then(|tid| self.turns_by_id.get(&format!("{tid}:a")).copied())
                } else {
                    self.cur_asst.or_else(|| self.f.last_turn_with_role(Role::Assistant))
                };
                // Old era fallback: a turn that ended with no agent_message at all
                // still gets its final text.
                if let (Some(t), Some(last)) = (t, str_of(p, "last_agent_message")) {
                    let has_text = self.f.turns[t].turn.blocks.iter().any(|b| matches!(b, Block::Text { .. }));
                    if !has_text && !last.trim().is_empty() {
                        self.text_block(t, last, &ts, idx);
                    }
                }
                if let Some(t) = t {
                    if let Some(ms) = ms {
                        self.f.turns[t].turn.duration_ms = Some(ms);
                        self.f.add_duration(ms);
                    }
                    self.f.touch(t, idx);
                    if str_of(p, "type") == Some("turn_aborted") {
                        let reason = str_of(p, "reason").unwrap_or("interrupted").to_string();
                        self.f.note(
                            Some(t),
                            SystemNote {
                                kind: SystemNoteKind::Other,
                                title: format!("Turn aborted ({reason})"),
                                body: None,
                            },
                            idx,
                        );
                    }
                }
                if !self.new_era {
                    self.cur_asst = None;
                }
            }
            Some("user_message") => {
                if self.new_era {
                    return; // duplicated by the UserMessage item
                }
                let text = str_of(p, "message").unwrap_or("");
                self.user_turn_old(text, ts, idx);
            }
            Some("agent_message") => {
                if self.new_era {
                    return; // duplicated by the AgentMessage item
                }
                let text = str_of(p, "message").unwrap_or("").to_string();
                let t = self.asst_turn(ts.clone(), idx);
                self.text_block(t, &text, &ts, idx);
            }
            Some("patch_apply_end") => {
                if self.new_era {
                    return; // duplicated by the FileChange item
                }
                let ok = p.get("success").and_then(Value::as_bool).unwrap_or(true);
                let changes = p.get("changes").cloned().unwrap_or(Value::Null);
                let (patch, first_path) = changes_to_patch(&changes);
                let stdout = str_of(p, "stdout").unwrap_or("");
                let mut result = Fold::result_from_text(ok, stdout, Vec::new());
                result.patch = patch;
                result.file_path = first_path.clone();
                let t = self.asst_turn(ts.clone(), idx);
                let turn_id = self.f.turns[t].turn.id.clone();
                // Enrich the call this belongs to: same `call_id` (custom_tool_call
                // apply_patch shares it), else the last pending function_call
                // apply_patch, else a block of its own.
                let call_id = str_of(p, "call_id").unwrap_or("").to_string();
                let target = self
                    .f
                    .tool_calls
                    .get(&call_id)
                    .copied()
                    .or_else(|| self.pending_patch_calls.pop());
                match target {
                    Some(r) => {
                        if let Some(slot) = self.f.result_mut(r) {
                            slot.patch = result.patch.clone();
                            slot.file_path = result.file_path.clone();
                            slot.ok = slot.ok && ok;
                            slot.cap();
                        } else if let Some(Block::ToolCall { result: slot, .. }) =
                            self.f.turns[r.turn].turn.blocks.get_mut(r.block)
                        {
                            *slot = Some(result);
                        }
                        self.f.touch(r.turn, idx);
                    }
                    None => {
                        let title = first_path.as_deref().map(|p| basename(p).to_string()).unwrap_or_else(|| "apply_patch".into());
                        self.f.push_tool_call(
                            t,
                            call_id,
                            "apply_patch".into(),
                            ToolKind::Edit,
                            title,
                            serde_json::json!({ "changes": changes }),
                            Some(result),
                            idx,
                        );
                    }
                }
                self.file_change_artifacts(&changes, &turn_id, &ts);
            }
            Some("mcp_tool_call_end") => {
                if self.new_era {
                    return;
                }
                let inv = p.get("invocation").cloned().unwrap_or(Value::Null);
                let server = str_of(&inv, "server").unwrap_or("mcp");
                let tool = str_of(&inv, "tool").unwrap_or("tool");
                let result = p.get("result").cloned().unwrap_or(Value::Null);
                let (ok, text) = mcp_result(&result);
                let t = self.asst_turn(ts.clone(), idx);
                self.f.push_tool_call(
                    t,
                    str_of(p, "call_id").unwrap_or("").to_string(),
                    format!("mcp__{server}__{tool}"),
                    ToolKind::Mcp,
                    format!("{server} · {tool}"),
                    inv.get("arguments").cloned().unwrap_or(Value::Null),
                    Some(Fold::result_from_text(ok, &text, Vec::new())),
                    idx,
                );
            }
            Some("web_search_end") => {
                if self.new_era {
                    return;
                }
                let q = str_of(p, "query").unwrap_or("").to_string();
                let t = self.asst_turn(ts.clone(), idx);
                self.f.push_tool_call(
                    t,
                    str_of(p, "call_id").unwrap_or("").to_string(),
                    "web_search".into(),
                    ToolKind::Web,
                    clip(&q, 120),
                    p.get("action").cloned().unwrap_or(Value::Null),
                    Some(Fold::result_from_text(true, "", Vec::new())),
                    idx,
                );
            }
            Some("image_generation_end") => {
                let t = self.asst_turn_any(ts.clone(), idx);
                self.f.push_tool_call(
                    t,
                    str_of(p, "call_id").unwrap_or("").to_string(),
                    "image_generation".into(),
                    ToolKind::Other,
                    "Generate image".into(),
                    serde_json::json!({ "revised_prompt": str_of(p, "revised_prompt").map(|s| clip(s, 500)) }),
                    Some(Fold::result_from_text(str_of(p, "status") != Some("failed"), "", Vec::new())),
                    idx,
                );
            }
            Some("context_compacted") => {
                if self.new_era {
                    return; // the ContextCompaction item carries it
                }
                let t = self.f.last_turn();
                self.f.note(
                    t,
                    SystemNote {
                        kind: SystemNoteKind::Compaction,
                        title: "Context compacted".into(),
                        body: None,
                    },
                    idx,
                );
            }
            Some("thread_settings_applied") => {
                if let Some(m) = p.get("thread_settings").and_then(model_of) {
                    self.f.model = Some(m);
                }
            }
            Some(other) => {
                // Progress/delta events we know exist but do not render.
                const QUIET: &[&str] = &[
                    "agent_message_delta",
                    "agent_reasoning",
                    "agent_reasoning_delta",
                    "reasoning_content_delta",
                    "exec_command_begin",
                    "exec_command_output_delta",
                    "exec_command_end",
                    "patch_apply_begin",
                    "mcp_tool_call_begin",
                    "web_search_begin",
                    "item_started",
                    "item_updated",
                    "turn_diff",
                    "background_event",
                    "exec_approval_request",
                    "apply_patch_approval_request",
                    "plan_update",
                    "session_configured",
                    "warning",
                    "error",
                    "stream_error",
                    "mcp_startup_update",
                    "mcp_list_tools_response",
                    "shutdown_complete",
                    "image_generation_begin",
                ];
                if QUIET.contains(&other) {
                    if matches!(other, "error" | "stream_error" | "warning") {
                        let t = self.f.last_turn();
                        self.f.note(
                            t,
                            SystemNote {
                                kind: SystemNoteKind::Other,
                                title: other.to_string(),
                                body: str_of(p, "message").map(|s| clip(s, 300)),
                            },
                            idx,
                        );
                    }
                    return;
                }
                self.f.unknown(&format!("event_msg/{other}"), idx);
            }
            None => self.f.unknown("event_msg/(untyped)", idx),
        }
    }

    /// Either era's "current assistant turn" for records that carry no turn id.
    fn asst_turn_any(&mut self, ts: Option<String>, idx: usize) -> usize {
        if self.new_era {
            match self.f.last_turn_with_role(Role::Assistant) {
                Some(t) => {
                    self.f.touch(t, idx);
                    t
                }
                None => {
                    let model = self.f.model.clone();
                    self.f.new_turn(format!("r{idx}"), Role::Assistant, ts, model, idx)
                }
            }
        } else {
            self.asst_turn(ts, idx)
        }
    }

    // ── new-era items ──────────────────────────────────────────────────────

    fn item(&mut self, idx: usize, v: &Value, p: &Value) {
        let ts = string_of(v, "timestamp");
        let turn_id = str_of(p, "turn_id").unwrap_or("").to_string();
        let item = p.get("item").cloned().unwrap_or(Value::Null);
        let item_id = str_of(&item, "id").unwrap_or("").to_string();
        match str_of(&item, "type") {
            Some("UserMessage") => {
                let text = content_text(item.get("content"));
                let t = self.turn_for(&turn_id, Role::User, ts.clone(), idx);
                if self.f.first_prompt.is_none() && !text.trim().is_empty() {
                    self.f.first_prompt = Some(clip(&text, 300));
                }
                self.f.push_block(t, Block::Text { md: text }, idx);
                // Pasted images travel as `local_images`/`images` paths (never
                // inline); surface them as read-only image artifacts by path.
                for key in ["images", "local_images"] {
                    if let Some(arr) = item.get(key).and_then(Value::as_array) {
                        let turn_key = self.f.turns[t].turn.id.clone();
                        for img in arr {
                            if let Some(pth) = img.as_str().or_else(|| str_of(img, "path")) {
                                self.f.artifact(ArtifactKind::Image, Some(pth.to_string()), None, &turn_key, ts.clone());
                            }
                        }
                    }
                }
            }
            Some("AgentMessage") => {
                let text = content_text(item.get("content"));
                let t = self.turn_for(&turn_id, Role::Assistant, ts.clone(), idx);
                self.text_block(t, &text, &ts, idx);
            }
            Some("Reasoning") => {
                self.f.stats.reasoning_steps += 1;
                // The assistant turn exists from here on so the footer has a home.
                let t = self.turn_for(&turn_id, Role::Assistant, ts, idx);
                self.f.turns[t].turn.reasoning_steps += 1;
            }
            Some("CommandExecution") => {
                let cmd = command_line(item.get("command"));
                let ok = match u64_of(&item, "exit_code") {
                    Some(code) => code == 0,
                    None => str_of(&item, "status") != Some("failed"),
                };
                let mut out = str_of(&item, "aggregated_output").unwrap_or("").to_string();
                if out.is_empty() {
                    out.push_str(str_of(&item, "stdout").unwrap_or(""));
                    if let Some(e) = str_of(&item, "stderr").filter(|e| !e.trim().is_empty()) {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(e);
                    }
                }
                let t = self.turn_for(&turn_id, Role::Assistant, ts, idx);
                self.f.push_tool_call(
                    t,
                    item_id,
                    "shell".into(),
                    ToolKind::Shell,
                    first_line(&cmd, 120),
                    serde_json::json!({ "command": cmd, "cwd": str_of(&item, "cwd"), "exit_code": item.get("exit_code") }),
                    Some(Fold::result_from_text(ok, &out, Vec::new())),
                    idx,
                );
            }
            Some("FileChange") => {
                let changes = item.get("changes").cloned().unwrap_or(Value::Null);
                let ok = str_of(&item, "status").is_none_or(|s| s != "failed");
                let t = self.turn_for(&turn_id, Role::Assistant, ts.clone(), idx);
                let turn_key = self.f.turns[t].turn.id.clone();
                let stdout = str_of(&item, "stdout").unwrap_or("");
                let mut any = false;
                if let Some(obj) = changes.as_object() {
                    for (n, (path, ch)) in obj.iter().enumerate() {
                        any = true;
                        let ctype = str_of(ch, "type").unwrap_or("update");
                        let kind = if ctype == "add" { ToolKind::Write } else { ToolKind::Edit };
                        let patch = str_of(ch, "unified_diff")
                            .map(|d| format!("--- a/{path}\n+++ b/{path}\n{d}"))
                            .or_else(|| str_of(ch, "content").map(|c| c.lines().map(|l| format!("+{l}")).collect::<Vec<_>>().join("\n")));
                        let mut result = Fold::result_from_text(ok, if n == 0 { stdout } else { "" }, Vec::new());
                        result.patch = patch;
                        result.file_path = Some(path.clone());
                        self.f.push_tool_call(
                            t,
                            if n == 0 { item_id.clone() } else { format!("{item_id}:{n}") },
                            "apply_patch".into(),
                            kind,
                            format!("{} {}", ctype, basename(path)),
                            serde_json::json!({ "path": path, "type": ctype }),
                            Some(result),
                            idx,
                        );
                    }
                }
                if !any {
                    self.f.push_tool_call(
                        t,
                        item_id,
                        "apply_patch".into(),
                        ToolKind::Edit,
                        "apply_patch".into(),
                        Value::Null,
                        Some(Fold::result_from_text(ok, stdout, Vec::new())),
                        idx,
                    );
                }
                self.file_change_artifacts(&changes, &turn_key, &ts);
            }
            Some("McpToolCall") => {
                let server = str_of(&item, "server").unwrap_or("mcp");
                let tool = str_of(&item, "tool").unwrap_or("tool");
                let result = item.get("result").cloned().unwrap_or(Value::Null);
                let (ok, text) = mcp_result(&result);
                let ok = ok && str_of(&item, "status") != Some("failed");
                let t = self.turn_for(&turn_id, Role::Assistant, ts, idx);
                self.f.push_tool_call(
                    t,
                    item_id,
                    format!("mcp__{server}__{tool}"),
                    ToolKind::Mcp,
                    format!("{server} · {tool}"),
                    item.get("arguments").cloned().unwrap_or(Value::Null),
                    Some(Fold::result_from_text(ok, &text, Vec::new())),
                    idx,
                );
            }
            Some("ContextCompaction") => {
                let t = self.turn_for(&turn_id, Role::Assistant, ts, idx);
                self.f.note(
                    Some(t),
                    SystemNote {
                        kind: SystemNoteKind::Compaction,
                        title: "Context compacted".into(),
                        body: None,
                    },
                    idx,
                );
            }
            Some("Extension") => {
                let kind = str_of(&item, "kind").unwrap_or("extension");
                let q = str_of(&item, "query").unwrap_or("").to_string();
                let n = item.get("results").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
                let t = self.turn_for(&turn_id, Role::Assistant, ts, idx);
                let tool = if kind.starts_with("web") { ToolKind::Web } else { ToolKind::Other };
                self.f.push_tool_call(
                    t,
                    item_id,
                    kind.to_string(),
                    tool,
                    if q.is_empty() { kind.to_string() } else { clip(&q, 120) },
                    item.get("action").cloned().unwrap_or(Value::Null),
                    Some(Fold::result_from_text(true, &format!("{n} result(s)"), Vec::new())),
                    idx,
                );
            }
            Some("ImageView") => {
                let path = str_of(&item, "path").unwrap_or("").trim_start_matches("file://").to_string();
                let t = self.turn_for(&turn_id, Role::Assistant, ts, idx);
                self.f.push_tool_call(
                    t,
                    item_id,
                    "view_image".into(),
                    ToolKind::Read,
                    basename(&path).to_string(),
                    serde_json::json!({ "path": path }),
                    Some(Fold::result_from_text(true, "", Vec::new())),
                    idx,
                );
            }
            Some(other) => self.f.unknown(&format!("item/{other}"), idx),
            None => self.f.unknown("item/(untyped)", idx),
        }
    }

    // ── response_item (old era only renders; new era = stats-only) ────────

    fn response_item(&mut self, idx: usize, v: &Value, p: &Value) {
        if self.new_era {
            return; // raw API log; every rendered fact came through an item
        }
        let ts = string_of(v, "timestamp");
        match str_of(p, "type") {
            Some("reasoning") => {
                self.f.stats.reasoning_steps += 1;
                let t = self.asst_turn(ts, idx);
                self.f.turns[t].turn.reasoning_steps += 1;
            }
            Some("message") => {
                let text = content_text(p.get("content"));
                match str_of(p, "role") {
                    Some("user") => {
                        if self.has_user_msg_events && !is_injected_context(&text) {
                            // The typed prompt also arrives as `event_msg
                            // user_message` — that one is rendered.
                            return;
                        }
                        if is_injected_context(&text) {
                            let t = self.f.last_turn();
                            self.f.note(
                                t,
                                SystemNote {
                                    kind: SystemNoteKind::SystemReminder,
                                    title: injected_title(&text),
                                    body: None,
                                },
                                idx,
                            );
                        } else {
                            self.user_turn_old(&text, ts, idx);
                        }
                    }
                    Some("assistant") => {
                        if self.has_agent_msg_events {
                            return;
                        }
                        let t = self.asst_turn(ts.clone(), idx);
                        self.text_block(t, &text, &ts, idx);
                    }
                    Some("developer") | Some("system") => {
                        let t = self.f.last_turn();
                        self.f.note(
                            t,
                            SystemNote {
                                kind: SystemNoteKind::SystemReminder,
                                title: injected_title(&text),
                                body: None,
                            },
                            idx,
                        );
                    }
                    other => self.f.unknown(&format!("message/{}", other.unwrap_or("?")), idx),
                }
            }
            Some("function_call") => {
                let name = str_of(p, "name").unwrap_or("tool").to_string();
                let args_raw = p.get("arguments").cloned().unwrap_or(Value::Null);
                let args = match &args_raw {
                    Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or(args_raw.clone()),
                    other => other.clone(),
                };
                let kind = tool_kind_for_codex_function(&name);
                let title = codex_function_title(&name, &args);
                let t = self.asst_turn(ts, idx);
                let r = self.f.push_tool_call(
                    t,
                    str_of(p, "call_id").unwrap_or("").to_string(),
                    name.clone(),
                    kind,
                    title,
                    args,
                    None,
                    idx,
                );
                if name == "apply_patch" {
                    self.pending_patch_calls.push(r);
                }
            }
            Some("function_call_output") | Some("custom_tool_call_output") => {
                let call_id = str_of(p, "call_id").unwrap_or("");
                let text = output_text(p.get("output"));
                let ok = !exit_code_failed(&text);
                let result = Fold::result_from_text(ok, &text, Vec::new());
                if self.f.attach_result(call_id, result, idx).is_none() {
                    let t = self.f.last_turn();
                    self.f.note(
                        t,
                        SystemNote {
                            kind: SystemNoteKind::Other,
                            title: "Orphan tool output".into(),
                            body: Some(clip(&text, 300)),
                        },
                        idx,
                    );
                }
            }
            Some("custom_tool_call") => {
                let name = str_of(p, "name").unwrap_or("exec").to_string();
                let input = str_of(p, "input").unwrap_or("").to_string();
                let t = self.asst_turn(ts, idx);
                let title = if name == "apply_patch" {
                    codex_function_title(&name, &serde_json::json!({ "input": input }))
                } else {
                    first_line(&input, 120)
                };
                let r = self.f.push_tool_call(
                    t,
                    str_of(p, "call_id").unwrap_or("").to_string(),
                    name.clone(),
                    if name == "exec" { ToolKind::Shell } else { tool_kind_for_codex_function(&name) },
                    title,
                    serde_json::json!({ "script": clip(&input, 4000) }),
                    None,
                    idx,
                );
                if name == "apply_patch" {
                    self.pending_patch_calls.push(r);
                }
            }
            Some("local_shell_call") => {
                let action = p.get("action").cloned().unwrap_or(Value::Null);
                let cmd = command_line(action.get("command"));
                let t = self.asst_turn(ts, idx);
                self.f.push_tool_call(
                    t,
                    str_of(p, "call_id").unwrap_or("").to_string(),
                    "shell".into(),
                    ToolKind::Shell,
                    first_line(&cmd, 120),
                    serde_json::json!({ "command": cmd }),
                    None,
                    idx,
                );
            }
            Some("web_search_call") | Some("image_generation_call") | Some("compaction") => {}
            Some(other) => self.f.unknown(&format!("response_item/{other}"), idx),
            None => self.f.unknown("response_item/(untyped)", idx),
        }
    }

    /// Register every changed path as an artifact.
    fn file_change_artifacts(&mut self, changes: &Value, turn_id: &str, ts: &Option<String>) {
        if let Some(obj) = changes.as_object() {
            for (path, ch) in obj {
                if str_of(ch, "type") == Some("delete") {
                    continue;
                }
                let kind = if crate::util::mime_for_path(path).is_some_and(|m| m.starts_with("image/")) {
                    ArtifactKind::Image
                } else {
                    ArtifactKind::File
                };
                self.f.artifact(kind, Some(path.clone()), None, turn_id, ts.clone());
            }
        }
    }
}

/// `payload.model` as a string, or `model.name|id|slug` for the object form.
fn model_of(v: &Value) -> Option<String> {
    match v.get("model")? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Object(_) => ["name", "id", "slug"].iter().find_map(|k| string_of(v.get("model")?, k)),
        _ => None,
    }
}

/// Join text parts of a content array (`text`, `Text`, `input_text`,
/// `output_text`); a bare string is returned as-is.
fn content_text(c: Option<&Value>) -> String {
    match c {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => {
            let mut out = String::new();
            for p in parts {
                let t = match p {
                    Value::String(s) => Some(s.as_str()),
                    Value::Object(_) => str_of(p, "text"),
                    _ => None,
                };
                if let Some(t) = t {
                    if !out.is_empty() {
                        out.push_str("\n\n");
                    }
                    out.push_str(t);
                }
            }
            out
        }
        _ => String::new(),
    }
}

/// Tool output: a string, or an array of `{type:"input_text"|"output_text", text}`.
fn output_text(o: Option<&Value>) -> String {
    match o {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(_)) => content_text(o),
        Some(Value::Object(obj)) => obj.get("text").or_else(|| obj.get("output")).and_then(Value::as_str).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// `["/bin/zsh","-lc","cmd"]` → `cmd`; other arrays are space-joined.
fn command_line(c: Option<&Value>) -> String {
    match c {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => {
            let parts: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
            if parts.len() >= 3 && parts[1] == "-lc" || parts.len() >= 3 && parts[1] == "-c" {
                parts[2..].join(" ")
            } else {
                parts.join(" ")
            }
        }
        _ => String::new(),
    }
}

/// `changes` (object keyed by absolute path) → one unified diff + first path.
fn changes_to_patch(changes: &Value) -> (Option<String>, Option<String>) {
    let Some(obj) = changes.as_object() else {
        return (None, None);
    };
    let mut out = String::new();
    let mut first = None;
    for (path, ch) in obj {
        if first.is_none() {
            first = Some(path.clone());
        }
        match str_of(ch, "type") {
            Some("add") => {
                out.push_str(&format!("--- /dev/null\n+++ b/{path}\n"));
                for l in str_of(ch, "content").unwrap_or("").lines() {
                    out.push('+');
                    out.push_str(l);
                    out.push('\n');
                }
            }
            Some("delete") => out.push_str(&format!("--- a/{path}\n+++ /dev/null\n")),
            _ => {
                out.push_str(&format!("--- a/{path}\n+++ b/{path}\n"));
                if let Some(d) = str_of(ch, "unified_diff") {
                    out.push_str(d);
                    if !d.ends_with('\n') {
                        out.push('\n');
                    }
                }
            }
        }
    }
    ((!out.is_empty()).then_some(out), first)
}

/// MCP result (`{Ok:{content,isError}}` or `{content,isError}`) → (ok, text).
fn mcp_result(r: &Value) -> (bool, String) {
    if let Some(err) = r.get("Err") {
        return (false, clip(&err.to_string(), 2000));
    }
    let inner = r.get("Ok").unwrap_or(r);
    let ok = !inner.get("isError").and_then(Value::as_bool).unwrap_or(false);
    (ok, content_text(inner.get("content")))
}

/// Codex `exec_command` outputs say `Process exited with code N`.
fn exit_code_failed(text: &str) -> bool {
    text.lines().take(8).any(|l| {
        l.trim()
            .strip_prefix("Process exited with code ")
            .is_some_and(|c| c.trim() != "0")
    })
}

fn is_injected_context(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("# AGENTS.md instructions")
        || t.starts_with("<environment_context>")
        || t.starts_with("<permissions")
        || t.starts_with("<collaboration_mode>")
        || t.starts_with("<INSTRUCTIONS>")
        || t.starts_with("<user_instructions>")
        || t.starts_with("<turn_aborted>")
        || t.starts_with("<skills_instructions>")
        || t.starts_with("<memory_citation")
}

fn injected_title(text: &str) -> String {
    let t = text.trim_start();
    if t.starts_with("# AGENTS.md") {
        return "AGENTS.md instructions".into();
    }
    if let Some(rest) = t.strip_prefix('<') {
        if let Some(end) = rest.find(['>', ' ', '\n']) {
            return rest[..end].replace('_', " ");
        }
    }
    clip(&first_line(t, 60), 60)
}

/// One-line title for an old-era `function_call`.
fn codex_function_title(name: &str, args: &Value) -> String {
    match name {
        "shell" | "exec_command" | "shell_command" | "container.exec" => {
            let cmd = args.get("cmd").or_else(|| args.get("command")).cloned().unwrap_or(Value::Null);
            let line = command_line(Some(&cmd));
            if line.is_empty() {
                name.to_string()
            } else {
                first_line(&line, 120)
            }
        }
        "apply_patch" => {
            let body = str_of(args, "input").or_else(|| str_of(args, "patch")).unwrap_or("");
            body.lines()
                .find_map(|l| l.strip_prefix("*** Update File: ").or_else(|| l.strip_prefix("*** Add File: ")))
                .map(|p| basename(p.trim()).to_string())
                .unwrap_or_else(|| "apply_patch".into())
        }
        "update_plan" => "Update plan".into(),
        "write_stdin" => "stdin".into(),
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::parse_records;

    fn fold(jsonl: &str) -> Folded {
        fold_codex(&parse_records(jsonl.as_bytes()), FoldOpts::default())
    }

    const NEW: &str = r##"{"timestamp":"2026-08-10T13:47:00Z","type":"session_meta","payload":{"session_id":"sid-1","cwd":"/repo","cli_version":"0.150.0","model":"gpt-5"}}
{"timestamp":"2026-08-10T13:47:01Z","type":"event_msg","payload":{"type":"task_started","turn_id":"T1"}}
{"timestamp":"2026-08-10T13:47:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"do it"}]}}
{"timestamp":"2026-08-10T13:47:02Z","ordinal":1,"type":"event_msg","payload":{"type":"item_completed","turn_id":"T1","item":{"type":"UserMessage","id":"um","content":[{"type":"text","text":"do it"}]}}}
{"timestamp":"2026-08-10T13:47:03Z","ordinal":2,"type":"event_msg","payload":{"type":"item_completed","turn_id":"T1","item":{"type":"Reasoning","id":"rs1","summary_text":[],"raw_content":[]}}}
{"timestamp":"2026-08-10T13:47:03Z","type":"response_item","payload":{"type":"reasoning","id":"rs1","summary":[]}}
{"timestamp":"2026-08-10T13:47:04Z","ordinal":3,"type":"event_msg","payload":{"type":"item_completed","turn_id":"T1","item":{"type":"CommandExecution","id":"exec-1","command":["/bin/zsh","-lc","ls"],"cwd":"file:///repo","status":"completed","stdout":"a\n","stderr":"","aggregated_output":"a\n","exit_code":0}}}
{"timestamp":"2026-08-10T13:47:05Z","ordinal":4,"type":"event_msg","payload":{"type":"item_completed","turn_id":"T1","item":{"type":"FileChange","id":"exec-2","changes":{"/repo/x.md":{"type":"update","unified_diff":"@@ -1 +1 @@\n-a\n+b\n"}},"status":"completed","stdout":"Success","stderr":""}}}
{"timestamp":"2026-08-10T13:47:06Z","ordinal":5,"type":"event_msg","payload":{"type":"item_completed","turn_id":"T1","item":{"type":"AgentMessage","id":"m1","content":[{"type":"Text","text":"Done."}]}}}
{"timestamp":"2026-08-10T13:47:06Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done."}]}}
{"timestamp":"2026-08-10T13:47:07Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":800,"output_tokens":50}}}}
{"timestamp":"2026-08-10T13:47:07Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"T1","last_agent_message":"Done.","duration_ms":6000}}
"##;

    #[test]
    fn new_era_renders_items_only() {
        let f = fold(NEW);
        assert_eq!(f.session_id.as_deref(), Some("sid-1"));
        assert_eq!(f.model.as_deref(), Some("gpt-5"));
        assert_eq!(f.turns.len(), 2, "response_item message/user must not add a turn");
        assert_eq!(f.turns[0].turn.id, "T1:u");
        assert_eq!(f.turns[1].turn.id, "T1:a");
        let a = &f.turns[1].turn;
        assert_eq!(a.duration_ms, Some(6000));
        let calls: Vec<&Block> = a.blocks.iter().filter(|b| matches!(b, Block::ToolCall { .. })).collect();
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[0], Block::ToolCall { tool: ToolKind::Shell, title, .. } if title == "ls"));
        assert!(matches!(calls[1], Block::ToolCall { tool: ToolKind::Edit, result: Some(r), .. } if r.patch.as_deref().unwrap().contains("+b")));
        let texts: Vec<&Block> = a.blocks.iter().filter(|b| matches!(b, Block::Text { .. })).collect();
        assert_eq!(texts.len(), 1, "response_item message/assistant must not duplicate the AgentMessage");
        assert_eq!(f.stats.reasoning_steps, 1, "response_item reasoning is not double counted");
        assert_eq!(a.reasoning_steps, 1);
        assert_eq!(f.stats.tool_calls, 2);
        assert_eq!(f.stats.input_tokens, Some(1000));
        assert_eq!(f.stats.output_tokens, Some(50));
        assert_eq!(f.stats.unknown_records, 0);
        assert_eq!(f.artifacts.len(), 1);
        assert_eq!(f.first_prompt.as_deref(), Some("do it"));
    }

    const OLD: &str = r##"{"timestamp":"2026-06-24T08:48:09Z","type":"session_meta","payload":{"id":"sid-old","cwd":"/repo","cli_version":"0.142.0"}}
{"timestamp":"2026-06-24T08:48:09Z","type":"event_msg","payload":{"type":"task_started","turn_id":"T1"}}
{"timestamp":"2026-06-24T08:48:09Z","type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<permissions instructions>\nstuff"}]}}
{"timestamp":"2026-06-24T08:48:09Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /repo\n..."}]}}
{"timestamp":"2026-06-24T08:48:09Z","type":"event_msg","payload":{"type":"user_message","message":"please start the app"}}
{"timestamp":"2026-06-24T08:48:14Z","type":"response_item","payload":{"type":"reasoning","id":"rs","summary":[]}}
{"timestamp":"2026-06-24T08:48:17Z","type":"event_msg","payload":{"type":"agent_message","message":"Looking."}}
{"timestamp":"2026-06-24T08:48:17Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Looking."}]}}
{"timestamp":"2026-06-24T08:48:17Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"rg --files\"}","call_id":"c1"}}
{"timestamp":"2026-06-24T08:48:17Z","type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"Process exited with code 0\nOutput:\nAGENTS.md"}}
{"timestamp":"2026-06-24T08:48:18Z","type":"response_item","payload":{"type":"function_call","name":"apply_patch","arguments":"{\"input\":\"*** Begin Patch\\n*** Update File: /repo/a.md\\n@@\\n-x\\n+y\\n*** End Patch\"}","call_id":"c2"}}
{"timestamp":"2026-06-24T08:48:18Z","type":"response_item","payload":{"type":"function_call_output","call_id":"c2","output":"Success"}}
{"timestamp":"2026-06-24T08:48:18Z","type":"event_msg","payload":{"type":"patch_apply_end","call_id":"exec-9","turn_id":"T1","stdout":"Success. Updated a.md","success":true,"changes":{"/repo/a.md":{"type":"update","unified_diff":"@@ -1 +1 @@\n-x\n+y"}}}}
{"timestamp":"2026-06-24T08:48:19Z","type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"const r = await tools.exec_command({cmd:'git status'})","call_id":"c3"}}
{"timestamp":"2026-06-24T08:48:19Z","type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"c3","output":[{"type":"input_text","text":"Script completed"},{"type":"input_text","text":"on main"}]}}
{"timestamp":"2026-06-24T10:15:21Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"T1","last_agent_message":"Fixed.","duration_ms":100}}
{"timestamp":"2026-06-24T10:15:22Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"cached_input_tokens":100,"output_tokens":20}}}}
"##;

    #[test]
    fn old_era_reads_events_and_function_calls() {
        let f = fold(OLD);
        assert_eq!(f.session_id.as_deref(), Some("sid-old"));
        assert_eq!(f.turns.len(), 2);
        let u = &f.turns[0].turn;
        assert!(u.id.starts_with('r'));
        assert!(matches!(&u.blocks[0], Block::Text { md } if md == "please start the app"));
        // developer + AGENTS.md injections became notes (on the user turn, as pending).
        assert_eq!(u.system.len(), 2);
        assert!(u.system.iter().any(|n| n.title == "AGENTS.md instructions"));
        let a = &f.turns[1].turn;
        let texts: Vec<&Block> = a.blocks.iter().filter(|b| matches!(b, Block::Text { .. })).collect();
        assert_eq!(texts.len(), 1, "agent_message wins over response_item message/assistant");
        let calls: Vec<&Block> = a.blocks.iter().filter(|b| matches!(b, Block::ToolCall { .. })).collect();
        assert_eq!(calls.len(), 3, "exec_command, apply_patch (enriched by patch_apply_end), exec");
        let Block::ToolCall { result: Some(r1), title, .. } = calls[0] else { panic!() };
        assert_eq!(title, "rg --files");
        assert!(r1.ok);
        let Block::ToolCall { result: Some(r2), tool, .. } = calls[1] else { panic!() };
        assert_eq!(*tool, ToolKind::Edit);
        assert_eq!(r2.file_path.as_deref(), Some("/repo/a.md"));
        assert!(r2.patch.as_deref().unwrap().contains("+y"));
        assert_eq!(r2.text.as_deref(), Some("Success"), "function output text kept");
        let Block::ToolCall { result: Some(r3), .. } = calls[2] else { panic!() };
        assert!(r3.text.as_deref().unwrap().contains("on main"));
        assert_eq!(a.duration_ms, Some(100));
        assert_eq!(f.stats.reasoning_steps, 1);
        assert_eq!(f.stats.input_tokens, Some(300));
        assert_eq!(f.stats.unknown_records, 0);
        assert_eq!(f.artifacts.len(), 1);
    }

    #[test]
    fn incremental_fold_matches_whole_fold_and_flags_era_flips() {
        let recs = parse_records(NEW.as_bytes());
        let whole = fold(NEW);
        let mut live = CodexFolder::new(FoldOpts::default());
        let mut refolds = 0;
        for r in &recs {
            if live.push(r) {
                refolds += 1;
            }
        }
        // The first item_completed arrives after session_meta etc. → one flip.
        assert!(refolds >= 1);
        // After a refold (prescan first) the live snapshot equals the whole fold.
        let mut live = CodexFolder::new(FoldOpts::default());
        for r in &recs {
            live.prescan(r);
        }
        for r in &recs {
            assert!(!live.push(r));
        }
        let snap = live.snapshot();
        assert_eq!(snap.turns.len(), whole.turns.len());
        assert_eq!(snap.stats, whole.stats);
        assert_eq!(live.record_count(), recs.len());
    }

    #[test]
    fn unknown_records_are_counted_not_dropped() {
        let f = fold("{\"type\":\"brand_new\"}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"never_seen\"}}\n");
        assert_eq!(f.stats.unknown_records, 2);
        assert_eq!(f.turns.len(), 1, "system-only file keeps its notices visible");
    }

    #[test]
    fn exit_code_detection() {
        assert!(exit_code_failed("Chunk ID: x\nProcess exited with code 1\nOutput:"));
        assert!(!exit_code_failed("Process exited with code 0\nfoo"));
        assert!(!exit_code_failed("plain"));
        assert_eq!(command_line(Some(&serde_json::json!(["/bin/zsh", "-lc", "ls -la"]))), "ls -la");
    }
}
