//! Auto-namer title helpers split out of `manager.rs` (LOC ratchet): turn a
//! provider transcript's first user prompt into a one-line session title.

use crate::manager::PROVIDER_TITLE_SCAN_LINES;

/// Max length (chars) of an auto-derived provider title before it is clipped.
pub(crate) const PROVIDER_TITLE_MAX: usize = 60;

/// Collapse a raw provider prompt into a one-line session title: strip control
/// characters / newlines to single spaces, squeeze runs of whitespace, and clip
/// to [`PROVIDER_TITLE_MAX`] chars (char-boundary safe, `…` suffix). Returns
/// `None` for an empty/blank result so callers skip it.
pub(crate) fn clean_provider_title(s: &str) -> Option<String> {
    let mut collapsed = String::with_capacity(s.len().min(256));
    let mut last_space = true; // leading whitespace is dropped
    for ch in s.chars() {
        if ch.is_whitespace() || ch.is_control() {
            if !last_space {
                collapsed.push(' ');
                last_space = true;
            }
        } else {
            collapsed.push(ch);
            last_space = false;
        }
    }
    let trimmed = collapsed.trim_end();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= PROVIDER_TITLE_MAX {
        return Some(trimmed.to_string());
    }
    let mut out: String = trimmed.chars().take(PROVIDER_TITLE_MAX).collect();
    // Don't leave a dangling space before the ellipsis.
    while out.ends_with(' ') {
        out.pop();
    }
    out.push('…');
    Some(out)
}

/// True when a claude/codex transcript text line is a wrapper/meta prompt rather
/// than a real user message: command echoes (`<command-name>…`), tool output
/// (`<local-command-stdout>`), injected context (`# AGENTS.md instructions`,
/// `<permissions instructions>`), or a Claude caveat preamble. These are never
/// the human's own words, so the title parsers skip them.
pub(crate) fn is_wrapper_prompt(text: &str) -> bool {
    let t = text.trim_start();
    t.is_empty()
        || t.starts_with('<')
        || t.starts_with("# AGENTS.md")
        || t.starts_with("Caveat:")
        || t.starts_with("# CLAUDE.md")
}

/// Pull the first genuine user prompt out of a Claude Code transcript JSONL
/// (`~/.claude/projects/<enc-cwd>/<uuid>.jsonl`). Claude records no dedicated
/// title/summary line in this format, so the first real `type:"user"` message is
/// the best human-meaningful name. Skips meta lines (`isMeta`), tool-result
/// user turns (array content with no text parts), and wrapper prompts
/// (slash-command echoes, injected CLAUDE.md/AGENTS.md). `None` when none is
/// found within [`PROVIDER_TITLE_SCAN_LINES`].
pub(crate) fn parse_claude_first_prompt(contents: &str) -> Option<String> {
    for line in contents.lines().take(PROVIDER_TITLE_SCAN_LINES) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("user") {
            continue;
        }
        // Sidechain (subagent) turns and injected meta turns aren't the user.
        if v.get("isMeta").and_then(|m| m.as_bool()) == Some(true)
            || v.get("isSidechain").and_then(|m| m.as_bool()) == Some(true)
        {
            continue;
        }
        let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
            continue;
        };
        let text = match content {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(parts) => {
                // Concatenate text parts; a tool_result-only turn yields "".
                let mut buf = String::new();
                for p in parts {
                    if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                            if !buf.is_empty() {
                                buf.push(' ');
                            }
                            buf.push_str(t);
                        }
                    }
                }
                buf
            }
            _ => continue,
        };
        if is_wrapper_prompt(&text) {
            continue;
        }
        if let Some(title) = clean_provider_title(&text) {
            return Some(title);
        }
    }
    None
}

/// Pull the first genuine user prompt out of a Codex rollout JSONL
/// (`rollout-<ts>-<uuid>.jsonl`). Codex records the typed prompt as an
/// `event_msg` of `payload.type == "user_message"` (`payload.message`) — the
/// clean human message, distinct from the injected AGENTS.md context that
/// arrives as a `response_item` user turn. Falls back to the first
/// `response_item` user `input_text` that isn't a wrapper prompt for older
/// rollouts that predate the `user_message` event. `None` when none is found
/// within [`PROVIDER_TITLE_SCAN_LINES`].
pub(crate) fn parse_codex_first_prompt(contents: &str) -> Option<String> {
    let mut fallback: Option<String> = None;
    for line in contents.lines().take(PROVIDER_TITLE_SCAN_LINES) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let payload = v.get("payload");
        match v.get("type").and_then(|t| t.as_str()) {
            Some("event_msg") => {
                let Some(p) = payload else { continue };
                if p.get("type").and_then(|t| t.as_str()) != Some("user_message") {
                    continue;
                }
                if let Some(msg) = p.get("message").and_then(|m| m.as_str()) {
                    if !is_wrapper_prompt(msg) {
                        if let Some(title) = clean_provider_title(msg) {
                            return Some(title);
                        }
                    }
                }
            }
            // Fallback for pre-`user_message` rollouts: first non-wrapper user
            // input_text. Recorded but not returned until the scan ends, so a
            // later real `user_message` event still wins.
            Some("response_item") if fallback.is_none() => {
                let Some(p) = payload else { continue };
                if p.get("type").and_then(|t| t.as_str()) != Some("message")
                    || p.get("role").and_then(|r| r.as_str()) != Some("user")
                {
                    continue;
                }
                if let Some(parts) = p.get("content").and_then(|c| c.as_array()) {
                    for part in parts {
                        if part.get("type").and_then(|t| t.as_str()) == Some("input_text") {
                            if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                                if !is_wrapper_prompt(t) {
                                    fallback = clean_provider_title(t);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fallback
}
