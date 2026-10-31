//! Antigravity (`agy`) JSONL transcript adapter:
//! `<brain_root>/<conversation-uuid>/.system_generated/logs/transcript_full.jsonl`
//! (preferred when non-empty) or `transcript.jsonl` → the normalized [`Folded`]
//! conversation model.

use std::collections::VecDeque;
use std::io::Read;
use std::path::Path;

use serde_json::Value;

use crate::fold::{Fold, FoldOpts, Folded};
use crate::model::*;
use crate::records::UNPARSEABLE_TYPE;
use crate::util::{basename, clip, first_line, pr_urls, str_of, u64_of};

/// Maximum `file://` media image bytes read into [`crate::ImageStore`] (10 MiB).
const MEDIA_IMAGE_CAP: u64 = 10 * 1024 * 1024;

/// Fold a whole `agy` transcript slice.
pub fn fold_agy(records: &[Value], opts: FoldOpts<'_>) -> Folded {
    let mut folder = AgyFolder::new(opts);
    for v in records {
        folder.push(v);
    }
    folder.into_folded()
}

/// Incremental `agy` folder for streaming / live tails.
#[derive(Clone)]
pub struct AgyFolder<'a> {
    st: AgyState<'a>,
    count: usize,
}

impl<'a> AgyFolder<'a> {
    pub fn new(opts: FoldOpts<'a>) -> Self {
        Self {
            st: AgyState {
                f: Fold::new(Provider::Agy, opts),
                cur_asst: None,
                pending_calls: VecDeque::new(),
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

    pub fn set_subagents(&mut self, subagents: Vec<SubagentMeta>) {
        self.st.f.opts.subagents = subagents;
    }

    pub fn subagents(&self) -> &[SubagentMeta] {
        self.st.f.subagents()
    }

    pub fn bounded_snapshot(&self, limit: usize, byte_limit: usize) -> Folded {
        self.st.f.window(self.count, None, limit, byte_limit)
    }

    pub fn page(
        &self,
        before: Option<usize>,
        limit: usize,
        subagents: Vec<SubagentMeta>,
    ) -> Transcript {
        let subagents = if subagents.is_empty() && before.is_none() {
            self.st.f.subagents().to_vec()
        } else {
            subagents
        };
        self.st.f.page(self.count, before, limit, subagents)
    }

    pub fn snapshot(&self) -> Folded {
        self.st.f.snapshot(self.count)
    }

    pub fn turns_since(&self, since: usize) -> Vec<Turn> {
        self.st.f.turns_since(since)
    }

    pub fn tool_block(&self, tool_id: &str) -> Option<Block> {
        self.st.f.tool_block(tool_id)
    }

    pub fn artifacts(&self) -> &[Artifact] {
        self.st.f.artifacts()
    }

    pub fn into_folded(self) -> Folded {
        self.st.f.finish(self.count)
    }
}

#[derive(Clone)]
struct PendingCall {
    id: String,
    bare_name: String,
    args: Value,
}

#[derive(Clone)]
struct AgyState<'a> {
    f: Fold<'a>,
    cur_asst: Option<usize>,
    pending_calls: VecDeque<PendingCall>,
}

impl<'a> AgyState<'a> {
    fn record(&mut self, idx: usize, v: &Value) {
        let Some(step_type) = str_of(v, "type") else {
            self.f.unknown("(no type)", idx);
            return;
        };
        if step_type == UNPARSEABLE_TYPE {
            self.f.unknown("unparseable JSON line", idx);
            return;
        }
        if step_type == "SYSTEM_MESSAGE" {
            return;
        }

        let ts = str_of(v, "created_at");
        match step_type {
            "USER_INPUT" => self.user_input(idx, v, ts),
            "PLANNER_RESPONSE" => self.planner_response(idx, v, ts),
            "GENERIC" => self.generic_result(idx, v, ts),
            "CHECKPOINT" | "EPHEMERAL_MESSAGE" => {}
            other => {
                if str_of(v, "source") != Some("SYSTEM") {
                    self.f.saw_ts(ts);
                    self.f.unknown(other, idx);
                }
            }
        }
    }

    fn user_input(&mut self, idx: usize, v: &Value, ts: Option<&str>) {
        let raw_content = str_of(v, "content").unwrap_or("");
        let cleaned = extract_user_prompt(raw_content);
        let media = v
            .get("media")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let has_image_media = media.iter().any(|m| {
            str_of(m, "mime_type")
                .or_else(|| str_of(m, "media_type"))
                .is_some_and(|mt| mt.starts_with("image/"))
                && str_of(m, "uri").is_some_and(|u| !u.is_empty())
        });
        if cleaned.is_empty() && !has_image_media {
            return;
        }

        self.f.saw_ts(ts);
        self.cur_asst = None;
        self.pending_calls.clear();

        let step_index = u64_of(v, "step_index").unwrap_or(idx as u64);
        let t = self.f.new_turn(
            format!("agy-u-{step_index}"),
            Role::User,
            ts.map(str::to_string),
            None,
            idx,
        );
        if self.f.first_prompt.is_none() && !cleaned.is_empty() {
            self.f.first_prompt = Some(clip(&cleaned, 240));
        }

        for item in media {
            let Some(mime) = str_of(item, "mime_type").or_else(|| str_of(item, "media_type"))
            else {
                continue;
            };
            let Some(uri) = str_of(item, "uri").filter(|u| !u.is_empty()) else {
                continue;
            };
            if !mime.starts_with("image/") {
                continue;
            }
            if let Some(file_path) = uri.strip_prefix("file://").map(percent_decode_path) {
                let alt = basename(&file_path).to_string();
                let alt_opt = (!alt.is_empty()).then(|| alt.clone());
                if let Some(store) = self.f.opts.images.clone() {
                    if let Some(bytes) = read_bounded_image(Path::new(&file_path)) {
                        let img_id = store.put_bytes(mime, &bytes);
                        self.f.push_block(
                            t,
                            Block::Image {
                                id: img_id,
                                media_type: mime.to_string(),
                                alt: alt_opt,
                            },
                            idx,
                        );
                        continue;
                    }
                }
                let label = if alt.is_empty() {
                    "image"
                } else {
                    alt.as_str()
                };
                self.f.push_block(
                    t,
                    Block::Text {
                        md: format!("![{label}]({uri})"),
                    },
                    idx,
                );
            } else {
                self.f.push_block(
                    t,
                    Block::Text {
                        md: format!("![image]({uri})"),
                    },
                    idx,
                );
            }
        }

        if !cleaned.is_empty() {
            self.f.push_block(t, Block::Text { md: cleaned }, idx);
        }
    }

    fn planner_response(&mut self, idx: usize, v: &Value, ts: Option<&str>) {
        self.f.saw_ts(ts);
        let step_index = u64_of(v, "step_index").unwrap_or(idx as u64);
        let step_model = str_of(v, "model")
            .and_then(|m| crate::agy::normalize_agy_model(m).or_else(|| Some(m.trim().to_string())))
            .filter(|m| !m.is_empty());
        if self.f.model.is_none() && step_model.is_some() {
            self.f.model.clone_from(&step_model);
        }

        let t = match self.cur_asst {
            Some(t) => {
                self.f.touch(t, idx);
                if self.f.turns[t].turn.model.is_none() && step_model.is_some() {
                    self.f.turns[t].turn.model.clone_from(&step_model);
                }
                t
            }
            None => {
                let turn_model = step_model.clone().or_else(|| self.f.model.clone());
                let t = self.f.new_turn(
                    format!("agy-a-{step_index}"),
                    Role::Assistant,
                    ts.map(str::to_string),
                    turn_model,
                    idx,
                );
                self.cur_asst = Some(t);
                t
            }
        };

        // Token usage (supports both top-level fields and nested `token_counts`).
        let tc = v.get("token_counts");
        let input = u64_of(v, "input_tokens")
            .or_else(|| tc.and_then(|c| u64_of(c, "input_tokens")))
            .unwrap_or(0);
        let output = u64_of(v, "output_tokens")
            .or_else(|| tc.and_then(|c| u64_of(c, "output_tokens")))
            .unwrap_or(0);
        let thinking_tokens = u64_of(v, "thinking_tokens")
            .or_else(|| tc.and_then(|c| u64_of(c, "thinking_tokens")))
            .unwrap_or(0);
        let cache_read = u64_of(v, "cache_read_tokens")
            .or_else(|| {
                tc.map(|c| {
                    u64_of(c, "cached_read_tokens")
                        .unwrap_or(0)
                        .saturating_add(u64_of(c, "ram_cache_read_tokens").unwrap_or(0))
                })
            })
            .unwrap_or(0);
        let cache_write = u64_of(v, "cache_write_tokens")
            .or_else(|| u64_of(v, "cache_creation_tokens"))
            .or_else(|| tc.and_then(|c| u64_of(c, "cache_write_tokens")))
            .unwrap_or(0);

        if input > 0 || output > 0 || thinking_tokens > 0 || cache_read > 0 || cache_write > 0 {
            let price_model = step_model
                .clone()
                .or_else(|| self.f.model.clone())
                .unwrap_or_else(|| "gemini-2.5-pro".to_string());
            self.f
                .usage(&price_model, input, output, cache_read, cache_write);
            self.f.turn_usage(
                Some(t),
                TurnUsage {
                    input_tokens: input,
                    output_tokens: output,
                    thinking_tokens,
                    cache_read_tokens: cache_read,
                    cache_creation_tokens: cache_write,
                },
                idx,
            );
        }

        if let Some(thinking) = str_of(v, "thinking")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            self.f.thinking_prose(t, thinking.to_string(), idx);
        }

        if let Some(calls) = v.get("tool_calls").and_then(Value::as_array) {
            for (call_idx, call) in calls.iter().enumerate() {
                let raw_name = str_of(call, "name").unwrap_or("tool");
                let bare_name = raw_name.strip_prefix("default_api:").unwrap_or(raw_name);
                let id = str_of(call, "id")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("agy-tc-{step_index}-{call_idx}"));
                let args = call
                    .get("args")
                    .or_else(|| call.get("arguments"))
                    .map(unquote_compact_args)
                    .unwrap_or_else(|| Value::Object(Default::default()));
                let kind = tool_kind_for_agy(raw_name);
                let title = agy_tool_title(bare_name, &args);
                self.f.push_tool_call(
                    t,
                    id.clone(),
                    bare_name.to_string(),
                    kind,
                    title,
                    args.clone(),
                    None,
                    idx,
                );

                // Register markdown artifacts written under `/brain/<sid>/`.
                if matches!(
                    bare_name,
                    "write_file"
                        | "write_to_file"
                        | "edit_file"
                        | "replace_file_content"
                        | "multi_replace_file_content"
                ) {
                    if let Some(path) = arg_str(&args, "Path")
                        .or_else(|| arg_str(&args, "TargetFile"))
                        .or_else(|| arg_str(&args, "path"))
                    {
                        if is_brain_markdown_artifact(&path) {
                            let turn_id = self.f.turns[t].turn.id.clone();
                            self.f.artifact(
                                ArtifactKind::File,
                                Some(path),
                                None,
                                &turn_id,
                                ts.map(str::to_string),
                            );
                        }
                    }
                }

                self.pending_calls.push_back(PendingCall {
                    id,
                    bare_name: bare_name.to_string(),
                    args,
                });
            }
        }

        if let Some(content) = str_of(v, "content").filter(|s| !s.trim().is_empty()) {
            let turn_id = self.f.turns[t].turn.id.clone();
            for url in pr_urls(content) {
                self.f.artifact(
                    ArtifactKind::Pr,
                    None,
                    Some(url),
                    &turn_id,
                    ts.map(str::to_string),
                );
            }
            self.f.push_block(
                t,
                Block::Text {
                    md: content.to_string(),
                },
                idx,
            );
        }
    }

    fn generic_result(&mut self, idx: usize, v: &Value, ts: Option<&str>) {
        self.f.saw_ts(ts);
        let explicit_id = str_of(v, "tool_call_id").or_else(|| str_of(v, "id"));
        let pending = match explicit_id.and_then(|wanted| {
            self.pending_calls
                .iter()
                .position(|p| p.id == wanted)
                .and_then(|pos| self.pending_calls.remove(pos))
        }) {
            Some(p) => p,
            None => match self.pending_calls.pop_front() {
                Some(p) => p,
                None => return,
            },
        };

        let is_error = str_of(v, "status") == Some("ERROR");
        let text = match str_of(v, "content").filter(|s| !s.is_empty()) {
            Some(c) => c.to_string(),
            None => str_of(v, "error").unwrap_or("").to_string(),
        };
        let mut res = Fold::result_from_text(!is_error, &text, Vec::new());
        res.file_path = arg_str(&pending.args, "Path")
            .or_else(|| arg_str(&pending.args, "TargetFile"))
            .or_else(|| arg_str(&pending.args, "path"));
        self.f.attach_result(&pending.id, res, idx);

        if pending.bare_name == "invoke_subagent" {
            if let Some(conv_id) = extract_conversation_id(&text)
                .or_else(|| str_of(v, "conversationId").map(str::to_string))
            {
                let description = arg_str(&pending.args, "Role")
                    .or_else(|| arg_str(&pending.args, "description"))
                    .or_else(|| arg_str(&pending.args, "Prompt"))
                    .map(|s| clip(&s, 120))
                    .unwrap_or_default();
                let agent_type = arg_str(&pending.args, "TypeName")
                    .or_else(|| arg_str(&pending.args, "subagent_type"))
                    .unwrap_or_else(|| "self".to_string());
                let model = arg_str(&pending.args, "Model").filter(|m| m != "inherit");
                if !self.f.opts.subagents.iter().any(|m| m.agent_id == conv_id) {
                    self.f.opts.subagents.push(SubagentMeta {
                        agent_id: conv_id,
                        parent_agent_id: None,
                        depth: 1,
                        agent_type,
                        description,
                        model,
                        tool_use_id: Some(pending.id),
                    });
                }
            }
        }
    }
}

/// Extract clean user text from an `agy` `USER_INPUT` content payload:
/// - If `<USER_REQUEST>...</USER_REQUEST>` tags are present, extract their inner text.
/// - Otherwise strip `<CONTEXT_SUMMARY>...</CONTEXT_SUMMARY>` and
///   `<SYSTEM_MESSAGE>...</SYSTEM_MESSAGE>` blocks (ignoring purely synthetic
///   compaction/system wrappers).
fn extract_user_prompt(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Some(extracted) = extract_tag_bodies(trimmed, "<USER_REQUEST>", "</USER_REQUEST>") {
        return extracted;
    }
    let without_ctx = strip_tagged_sections(trimmed, "<CONTEXT_SUMMARY>", "</CONTEXT_SUMMARY>");
    let without_sys = strip_tagged_sections(&without_ctx, "<SYSTEM_MESSAGE>", "</SYSTEM_MESSAGE>");
    let cleaned = without_sys.trim();
    // Ignore the canned preamble agy places immediately above `<SYSTEM_MESSAGE>`.
    if cleaned.is_empty()
        || cleaned.starts_with("The following is a <SYSTEM_MESSAGE>")
        || cleaned.starts_with("The following is a summary of the conversation history")
    {
        return String::new();
    }
    cleaned.to_string()
}

fn extract_tag_bodies(text: &str, open: &str, close: &str) -> Option<String> {
    let mut parts = Vec::new();
    let mut rest = text;
    let mut found = false;
    while let Some(start) = rest.find(open) {
        found = true;
        let after_open = &rest[start + open.len()..];
        match after_open.find(close) {
            Some(end) => {
                let inner = after_open[..end].trim();
                if !inner.is_empty() {
                    parts.push(inner);
                }
                rest = &after_open[end + close.len()..];
            }
            None => {
                let inner = after_open.trim();
                if !inner.is_empty() {
                    parts.push(inner);
                }
                break;
            }
        }
    }
    found.then(|| parts.join("\n\n"))
}

fn strip_tagged_sections(text: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + open.len()..];
        match after_open.find(close) {
            Some(end) => rest = &after_open[end + close.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Compact `transcript.jsonl` stringifies individual `args` values as JSON
/// literals (e.g. `"\"/path/to/file\""`). Unwrap those one layer so titles and
/// paths match `transcript_full.jsonl`.
fn unquote_compact_args(v: &Value) -> Value {
    let Some(obj) = v.as_object() else {
        return v.clone();
    };
    let mut out = serde_json::Map::with_capacity(obj.len());
    for (k, val) in obj {
        let normalized = match val.as_str() {
            Some(s) if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') => {
                serde_json::from_str::<Value>(s).unwrap_or_else(|_| val.clone())
            }
            _ => val.clone(),
        };
        out.insert(k.clone(), normalized);
    }
    Value::Object(out)
}

fn arg_str(args: &Value, key: &str) -> Option<String> {
    str_of(args, key)
        .map(|s| s.trim_matches('"').trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Human-readable one-line title for an `agy` tool call.
pub fn agy_tool_title(bare_name: &str, args: &Value) -> String {
    let s = |k: &str| arg_str(args, k).unwrap_or_default();
    match bare_name {
        "run_command" => {
            let summary = s("toolSummary");
            if !summary.is_empty() {
                clip(&summary, 120)
            } else {
                let cmd = s("CommandLine");
                if !cmd.is_empty() {
                    first_line(&cmd, 120)
                } else {
                    bare_name.to_string()
                }
            }
        }
        "read_file"
        | "view_file"
        | "view_file_outline"
        | "edit_file"
        | "write_file"
        | "write_to_file"
        | "replace_file_content"
        | "multi_replace_file_content" => {
            let p = ["Path", "TargetFile", "AbsolutePath", "path"]
                .into_iter()
                .map(s)
                .find(|v| !v.is_empty())
                .unwrap_or_default();
            if p.is_empty() {
                bare_name.to_string()
            } else {
                basename(&p).to_string()
            }
        }
        "grep_search" | "find_by_name" | "codebase_search" => {
            let q = ["Query", "Pattern", "query", "pattern"]
                .into_iter()
                .map(s)
                .find(|v| !v.is_empty())
                .unwrap_or_default();
            if q.is_empty() {
                bare_name.to_string()
            } else {
                format!("{bare_name} {}", clip(&q, 100))
            }
        }
        "invoke_subagent" => {
            let role = s("Role");
            if !role.is_empty() {
                clip(&role, 120)
            } else {
                let t = s("TypeName");
                if t.is_empty() {
                    bare_name.to_string()
                } else {
                    clip(&t, 80)
                }
            }
        }
        "call_mcp_tool" => {
            let srv = s("ServerName");
            let tool = s("ToolName");
            if !srv.is_empty() && !tool.is_empty() {
                format!("{srv}.{tool}")
            } else if !tool.is_empty() {
                tool
            } else {
                bare_name.to_string()
            }
        }
        "search_web" => {
            let q = s("query");
            if q.is_empty() {
                bare_name.to_string()
            } else {
                format!("search_web {}", clip(&q, 100))
            }
        }
        _ => bare_name.to_string(),
    }
}

fn is_brain_markdown_artifact(path: &str) -> bool {
    path.contains("/brain/")
        && path.ends_with(".md")
        && !path.contains("/.system_generated/")
        && !path.contains("/scratch/")
}

/// Extract `conversationId` from an `invoke_subagent` tool result (either pure
/// JSON or prefixed with `Created At: ...` lines).
fn extract_conversation_id(text: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        if let Some(id) = str_of(&v, "conversationId").or_else(|| str_of(&v, "conversation_id")) {
            if is_safe_conv_id(id) {
                return Some(id.to_string());
            }
        }
    }
    for needle in [
        "\"conversationId\"",
        "conversationId",
        "\"conversation_id\"",
    ] {
        let Some(pos) = text.find(needle) else {
            continue;
        };
        let rest = &text[pos + needle.len()..];
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix(':').or_else(|| rest.strip_prefix('=')) else {
            continue;
        };
        let rest = rest.trim_start().trim_start_matches('"');
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len());
        let candidate = &rest[..end];
        if is_safe_conv_id(candidate) {
            return Some(candidate.to_string());
        }
    }
    None
}

fn is_safe_conv_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn percent_decode_path(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| raw.to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[allow(clippy::disallowed_methods)] // sync fold helper — callers run off the async runtime
fn read_bounded_image(path: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MEDIA_IMAGE_CAP {
        return None;
    }
    let mut buf = Vec::with_capacity(meta.len() as usize);
    std::fs::File::open(path)
        .ok()?
        .take(MEDIA_IMAGE_CAP + 1)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.is_empty() || buf.len() as u64 > MEDIA_IMAGE_CAP {
        return None;
    }
    Some(buf)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::images::ImageStore;
    use crate::subagents::subagent_path_for;

    #[test]
    fn skips_context_summary_and_system_messages_and_extracts_user_request() {
        let records = vec![
            json!({
                "step_index": 0,
                "source": "SYSTEM",
                "type": "SYSTEM_MESSAGE",
                "status": "DONE",
                "created_at": "2026-10-31T15:00:00Z",
                "content": "<SYSTEM_MESSAGE>backgroundwakeup</SYSTEM_MESSAGE>"
            }),
            json!({
                "step_index": 1,
                "source": "USER_EXPLICIT",
                "type": "USER_INPUT",
                "status": "DONE",
                "created_at": "2026-10-31T15:00:01Z",
                "content": "<CONTEXT_SUMMARY>\nold summary\n</CONTEXT_SUMMARY>"
            }),
            json!({
                "step_index": 2,
                "source": "USER_EXPLICIT",
                "type": "USER_INPUT",
                "status": "DONE",
                "created_at": "2026-10-31T15:00:02Z",
                "content": "<CONTEXT_SUMMARY>prior</CONTEXT_SUMMARY>\n<USER_REQUEST>\nFix the login bug\n</USER_REQUEST>"
            }),
        ];
        let folded = fold_agy(&records, FoldOpts::default());
        assert_eq!(folded.turns.len(), 1);
        assert_eq!(folded.stats.turns, 1);
        assert_eq!(folded.first_prompt.as_deref(), Some("Fix the login bug"));
        let u = &folded.turns[0].turn;
        assert_eq!(u.id, "agy-u-2");
        assert_eq!(u.role, Role::User);
        assert!(matches!(&u.blocks[0], Block::Text { md } if md == "Fix the login bug"));
    }

    #[test]
    fn coalesces_planner_responses_thinking_prose_tools_usage_and_cost() {
        let price = |_model: &str, inp: u64, out: u64, cr: u64, cw: u64| -> f64 {
            (inp as f64) * 0.001
                + (out as f64) * 0.002
                + (cr as f64) * 0.0001
                + (cw as f64) * 0.0005
        };
        let records = vec![
            json!({
                "step_index": 0,
                "source": "USER_EXPLICIT",
                "type": "USER_INPUT",
                "status": "DONE",
                "created_at": "2026-10-31T15:10:00Z",
                "content": "<USER_REQUEST>Read Cargo.toml and build</USER_REQUEST>"
            }),
            json!({
                "step_index": 1,
                "source": "MODEL",
                "type": "PLANNER_RESPONSE",
                "status": "DONE",
                "created_at": "2026-10-31T15:10:02Z",
                "model": "gemini-2.5-pro",
                "thinking": "  First let's inspect Cargo.toml. \n",
                "input_tokens": 1000,
                "cache_read_tokens": 200,
                "cache_write_tokens": 50,
                "output_tokens": 120,
                "thinking_tokens": 40,
                "tool_calls": [
                    {
                        "name": "default_api:read_file",
                        "args": { "Path": "/repo/Cargo.toml" }
                    },
                    {
                        "name": "run_command",
                        "args": { "CommandLine": "cargo check", "toolSummary": "Check workspace" }
                    }
                ]
            }),
            json!({
                "step_index": 2,
                "source": "MODEL",
                "type": "GENERIC",
                "status": "DONE",
                "created_at": "2026-10-31T15:10:03Z",
                "content": "[package]\nname = \"demo\""
            }),
            json!({
                "step_index": 3,
                "source": "MODEL",
                "type": "GENERIC",
                "status": "ERROR",
                "created_at": "2026-10-31T15:10:04Z",
                "content": "compile error: missing semicolon"
            }),
            json!({
                "step_index": 4,
                "source": "MODEL",
                "type": "PLANNER_RESPONSE",
                "status": "DONE",
                "created_at": "2026-10-31T15:10:06Z",
                "thinking": "Found the missing semicolon.",
                "input_tokens": 500,
                "output_tokens": 80,
                "thinking_tokens": 10,
                "tool_calls": [
                    {
                        "name": "write_file",
                        "args": { "Path": "/Users/dev/.gemini/antigravity-cli/brain/conv-1/plan.md", "Content": "# Plan" }
                    }
                ],
                "content": "All set! Created the plan."
            }),
            json!({
                "step_index": 5,
                "source": "MODEL",
                "type": "GENERIC",
                "status": "DONE",
                "created_at": "2026-10-31T15:10:07Z",
                "content": "Wrote plan.md"
            }),
        ];

        let folded = fold_agy(
            &records,
            FoldOpts {
                price: Some(&price),
                ..FoldOpts::default()
            },
        );
        // 1 user turn + 1 coalesced assistant turn.
        assert_eq!(folded.turns.len(), 2);
        assert_eq!(folded.stats.turns, 2);
        assert_eq!(folded.stats.tool_calls, 3);
        assert_eq!(folded.stats.thinking_steps, 2);
        assert_eq!(folded.stats.input_tokens, Some(1000 + 200 + 50 + 500));
        assert_eq!(folded.stats.output_tokens, Some(120 + 80));
        assert!(folded.stats.cost_usd.unwrap() > 1.9);
        assert_eq!(folded.artifacts.len(), 1);
        assert_eq!(folded.artifacts[0].label, "plan.md");

        let asst = &folded.turns[1].turn;
        assert_eq!(asst.id, "agy-a-1");
        assert_eq!(asst.role, Role::Assistant);
        assert_eq!(
            asst.usage,
            Some(TurnUsage {
                input_tokens: 1500,
                output_tokens: 200,
                thinking_tokens: 50,
                cache_read_tokens: 200,
                cache_creation_tokens: 50,
            })
        );
        assert!(matches!(
            &asst.blocks[0],
            Block::Thinking {
                count: 1,
                text: Some(t)
            } if t == "First let's inspect Cargo.toml."
        ));
        let Block::ToolCall {
            name,
            tool,
            title,
            result: Some(r1),
            ..
        } = &asst.blocks[1]
        else {
            panic!("expected first tool call");
        };
        assert_eq!(name, "read_file");
        assert_eq!(*tool, ToolKind::Read);
        assert_eq!(title, "Cargo.toml");
        assert!(r1.ok);
        assert_eq!(r1.text.as_deref(), Some("[package]\nname = \"demo\""));

        let Block::ToolCall {
            tool: tool2,
            title: title2,
            result: Some(r2),
            ..
        } = &asst.blocks[2]
        else {
            panic!("expected second tool call");
        };
        assert_eq!(*tool2, ToolKind::Shell);
        assert_eq!(title2, "Check workspace");
        assert!(!r2.ok);
        assert_eq!(r2.text.as_deref(), Some("compile error: missing semicolon"));
    }

    #[test]
    fn invoke_subagent_emits_subagent_block_meta_and_resolves_sub_path() {
        let tmp = tempfile::tempdir().unwrap();
        let brain = tmp.path().join("brain");
        let parent_id = "11111111-1111-1111-1111-111111111111";
        let child_id = "22222222-2222-2222-2222-222222222222";
        let parent_log = brain
            .join(parent_id)
            .join(".system_generated/logs/transcript_full.jsonl");
        let child_log = brain
            .join(child_id)
            .join(".system_generated/logs/transcript_full.jsonl");
        std::fs::create_dir_all(parent_log.parent().unwrap()).unwrap();
        std::fs::create_dir_all(child_log.parent().unwrap()).unwrap();
        std::fs::write(&parent_log, b"{}\n").unwrap();
        std::fs::write(
            &child_log,
            b"{\"step_index\":0,\"type\":\"USER_INPUT\",\"content\":\"hi\"}\n",
        )
        .unwrap();

        let records = vec![
            json!({
                "step_index": 0,
                "source": "USER_EXPLICIT",
                "type": "USER_INPUT",
                "status": "DONE",
                "content": "Delegate research"
            }),
            json!({
                "step_index": 1,
                "source": "MODEL",
                "type": "PLANNER_RESPONSE",
                "status": "DONE",
                "tool_calls": [
                    {
                        "id": "call_sub_1",
                        "name": "invoke_subagent",
                        "args": {
                            "TypeName": "research",
                            "Role": "Auth Flow Researcher",
                            "Prompt": "Inspect auth middleware"
                        }
                    }
                ]
            }),
            json!({
                "step_index": 2,
                "source": "MODEL",
                "type": "GENERIC",
                "status": "DONE",
                "content": format!("Created At: 2026-10-31T15:00:00Z\n{{\"conversationId\": \"{child_id}\", \"status\": \"running\"}}")
            }),
        ];

        let folded = fold_agy(&records, FoldOpts::default());
        assert_eq!(folded.subagents.len(), 1);
        assert_eq!(folded.subagents[0].agent_id, child_id);
        assert_eq!(folded.subagents[0].agent_type, "research");
        assert_eq!(folded.subagents[0].description, "Auth Flow Researcher");

        let blocks = &folded.turns[1].turn.blocks;
        assert!(blocks.iter().any(|b| matches!(
            b,
            Block::Subagent {
                agent_id,
                description,
                agent_type,
                status: Some(SubagentStatus::Done),
            } if agent_id == child_id && description == "Auth Flow Researcher" && agent_type == "research"
        )));

        // `?sub=<uuid>` resolves to the child's brain transcript_full.jsonl.
        let resolved = subagent_path_for(Provider::Agy, &parent_log, child_id).unwrap();
        assert_eq!(resolved, child_log);
        assert!(subagent_path_for(Provider::Agy, &parent_log, "../escape").is_none());
    }

    #[test]
    fn user_media_image_extracts_into_image_store_or_markdown_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = tmp.path().join("screen.png");
        let png_bytes = b"\x89PNG\r\n\x1a\nfake";
        std::fs::write(&img_path, png_bytes).unwrap();
        let uri = format!("file://{}", img_path.display());

        let records = vec![json!({
            "step_index": 0,
            "source": "USER_EXPLICIT",
            "type": "USER_INPUT",
            "status": "DONE",
            "content": "<USER_REQUEST>Check screenshot</USER_REQUEST>",
            "media": [{ "mime_type": "image/png", "uri": uri }]
        })];

        // Without ImageStore -> markdown image block.
        let no_store = fold_agy(&records, FoldOpts::default());
        assert!(matches!(
            &no_store.turns[0].turn.blocks[0],
            Block::Text { md } if md.starts_with("![screen.png](file://")
        ));

        // With ImageStore -> persisted and emitted as Block::Image.
        let store = ImageStore::new(tmp.path().join("store"));
        let with_store = fold_agy(
            &records,
            FoldOpts {
                images: Some(store.clone()),
                ..FoldOpts::default()
            },
        );
        let Block::Image {
            id,
            media_type,
            alt,
        } = &with_store.turns[0].turn.blocks[0]
        else {
            panic!("expected Block::Image");
        };
        assert_eq!(media_type, "image/png");
        assert_eq!(alt.as_deref(), Some("screen.png"));
        assert!(store.find(id).is_some());
    }
}
