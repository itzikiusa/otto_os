//! Small tolerant helpers shared by the Claude and Codex adapters. Everything
//! here takes `serde_json::Value` and never panics on a missing/odd field —
//! transcript schemas drift between CLI versions and 1.5% of `toolUseResult`s
//! are bare strings.

use serde_json::Value;

use crate::model::{SystemNote, SystemNoteKind};

/// Cap on a tool result's text (design §3: "Tool result text capped at 64 KB").
pub const TOOL_TEXT_CAP: usize = 64 * 1024;

/// `obj[key]` as `&str`, tolerant of a missing key or a non-string.
pub fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// `obj[key]` as an owned `String`.
pub fn string_of(v: &Value, key: &str) -> Option<String> {
    str_of(v, key).map(str::to_string)
}

/// `obj[key]` as `u64` (accepts a float that is a whole number, as JS emits).
/// Non-finite, negative or `> u64::MAX` values are rejected (`None`) rather
/// than saturated — a corrupt `1e300` token count must not become 2^64.
pub fn u64_of(v: &Value, key: &str) -> Option<u64> {
    match v.get(key)? {
        Value::Number(n) => n.as_u64().or_else(|| {
            let f = n.as_f64()?;
            (f.is_finite() && f >= 0.0 && f < u64::MAX as f64).then_some(f as u64)
        }),
        _ => None,
    }
}

/// Cap on a serialized `ToolCall.input` (bytes); larger inputs are replaced by
/// a preview object so one pasted blob cannot dominate a page.
pub const TOOL_INPUT_CAP: usize = 16 * 1024;

/// Clip an oversized tool input to `{"_truncated": true, "bytes", "preview"}`.
pub fn clip_input(input: Value) -> Value {
    let raw = input.to_string();
    if raw.len() <= TOOL_INPUT_CAP {
        return input;
    }
    serde_json::json!({ "_truncated": true, "bytes": raw.len(), "preview": clip(&raw, 4000) })
}

/// Truncate `s` to at most `max` chars (char-boundary safe), appending `…`.
pub fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// First non-empty line of `s`, clipped to `max` chars.
pub fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    clip(line, max)
}

/// Last path segment (terser titles for file tools).
pub fn basename(p: &str) -> &str {
    p.rsplit(['/', '\\']).next().unwrap_or(p)
}

/// Cap `text` at [`TOOL_TEXT_CAP`] bytes (on a char boundary). Returns the
/// (possibly shortened) text and whether it was cut.
pub fn cap_text(text: &str) -> (String, bool) {
    if text.len() <= TOOL_TEXT_CAP {
        return (text.to_string(), false);
    }
    let mut end = TOOL_TEXT_CAP;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}\n… [truncated]", &text[..end]), true)
}

/// Claude pseudo-tags embedded in user text. Each `<tag>…</tag>` span is
/// removed from the prose and turned into a collapsed [`SystemNote`].
const PSEUDO_TAGS: &[(&str, SystemNoteKind)] = &[
    ("system-reminder", SystemNoteKind::SystemReminder),
    ("task-notification", SystemNoteKind::TaskNotification),
    ("local-command-stdout", SystemNoteKind::Command),
    ("local-command-caveat", SystemNoteKind::Command),
    ("command-message", SystemNoteKind::Command),
    ("command-args", SystemNoteKind::Command),
    ("command-name", SystemNoteKind::Command),
];

/// Strip Claude's pseudo-tags out of user prose, returning the clean text plus
/// one note per span. `<command-name>/x</command-name><command-args>y</command-args>`
/// folds into a single `command` note titled `/x y`. Unterminated tags are
/// left in place (the text is shown as-is rather than eaten).
pub fn extract_pseudo_tags(text: &str) -> (String, Vec<SystemNote>) {
    let mut out = text.to_string();
    let mut notes = Vec::new();
    let mut command_name: Option<String> = None;
    let mut command_args: Option<String> = None;
    for (tag, kind) in PSEUDO_TAGS {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        while let Some(start) = out.find(&open) {
            let Some(rel_end) = out[start + open.len()..].find(&close) else { break };
            let body_start = start + open.len();
            let body_end = body_start + rel_end;
            let body = out[body_start..body_end].trim().to_string();
            out.replace_range(start..body_end + close.len(), "");
            match *tag {
                "command-name" => command_name = Some(body),
                "command-args" => command_args = Some(body),
                _ => notes.push(SystemNote {
                    kind: *kind,
                    title: pseudo_title(tag, &body),
                    body: (!body.is_empty()).then(|| clip(&body, 4000)),
                }),
            }
        }
    }
    if let Some(name) = command_name {
        let title = match command_args.as_deref().filter(|a| !a.is_empty()) {
            Some(args) => format!("{name} {}", clip(args, 120)),
            None => name,
        };
        notes.insert(
            0,
            SystemNote {
                kind: SystemNoteKind::Command,
                title,
                body: None,
            },
        );
    }
    // Collapse the whitespace the removed spans leave behind.
    let clean = out
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    (clean, notes)
}

fn pseudo_title(tag: &str, body: &str) -> String {
    match tag {
        "system-reminder" => "System reminder".to_string(),
        "task-notification" => {
            // `<summary>…</summary>` inside the notification is the human line.
            inner_tag(body, "summary")
                .map(|s| format!("Task: {}", clip(&s, 120)))
                .unwrap_or_else(|| "Task notification".to_string())
        }
        "local-command-stdout" => "Command output".to_string(),
        "local-command-caveat" => "Command caveat".to_string(),
        "command-message" => clip(body, 120),
        _ => tag.to_string(),
    }
}

/// The text between `<tag>` and `</tag>` inside `s`, if both are present.
pub fn inner_tag(s: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let end = start + s[start..].find(&close)?;
    Some(s[start..end].trim().to_string())
}

/// Render Claude's `structuredPatch` (`[{oldStart, oldLines, newStart,
/// newLines, lines: ["-a", "+b", " ctx"]}]`) as a unified diff body so the UI
/// can feed it to the existing `DiffViewer`. Returns `None` when there are no
/// hunks (a fresh `Write` has `structuredPatch: []`).
pub fn structured_patch_to_unified(patch: &Value, file: Option<&str>) -> Option<String> {
    let hunks = patch.as_array()?;
    if hunks.is_empty() {
        return None;
    }
    let mut out = String::new();
    if let Some(f) = file {
        out.push_str(&format!("--- a/{f}\n+++ b/{f}\n"));
    }
    for h in hunks {
        let os = u64_of(h, "oldStart").unwrap_or(0);
        let ol = u64_of(h, "oldLines").unwrap_or(0);
        let ns = u64_of(h, "newStart").unwrap_or(0);
        let nl = u64_of(h, "newLines").unwrap_or(0);
        out.push_str(&format!("@@ -{os},{ol} +{ns},{nl} @@\n"));
        if let Some(lines) = h.get("lines").and_then(Value::as_array) {
            for l in lines {
                if let Some(s) = l.as_str() {
                    out.push_str(s);
                    out.push('\n');
                }
            }
        }
    }
    Some(out)
}

/// Pull-request URLs mentioned in prose (GitHub `/pull/N`, Bitbucket
/// `/pull-requests/N`, GitLab `/merge_requests/N`). Cheap whitespace/punctuation
/// tokenizer — no regex crate in the tree.
pub fn pr_urls(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tok in text.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>' | '"' | '\'' | '`' | ',')) {
        let tok = tok.trim_end_matches(['.', ';', ':', ']', '*']);
        if !tok.starts_with("https://") {
            continue;
        }
        let is_pr = tok.contains("/pull/") || tok.contains("/pull-requests/") || tok.contains("/merge_requests/");
        if is_pr
            && tok
                .rsplit('/')
                .next()
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            && !out.iter().any(|o| o == tok)
        {
            out.push(tok.to_string());
        }
    }
    out
}

/// Short human label for a PR url: `owner/repo#N` when the shape is known.
pub fn pr_label(url: &str) -> String {
    let path = url.trim_start_matches("https://");
    let parts: Vec<&str> = path.split('/').collect();
    let num = parts.last().copied().unwrap_or("");
    if parts.len() >= 3 {
        return format!("{}/{}#{num}", parts[1], parts[2]);
    }
    format!("PR #{num}")
}

/// Guess a MIME type from a path's extension (previewable artifact kinds only).
pub fn mime_for_path(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "html" | "htm" => "text/html",
        "md" | "markdown" => "text/markdown",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "csv" => "text/csv",
        "json" => "application/json",
        "txt" | "log" => "text/plain",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pseudo_tags_become_notes_and_leave_clean_prose() {
        let text = "hello\n<system-reminder>\nremember X\n</system-reminder>\nworld";
        let (clean, notes) = extract_pseudo_tags(text);
        assert_eq!(clean, "hello\n\nworld");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].kind, SystemNoteKind::SystemReminder);
        assert_eq!(notes[0].body.as_deref(), Some("remember X"));
    }

    #[test]
    fn command_tags_fold_into_one_note() {
        let text = "<command-name>/commit</command-name>\n<command-message>commit</command-message>\n<command-args>-m x</command-args>";
        let (clean, notes) = extract_pseudo_tags(text);
        assert!(clean.is_empty());
        assert_eq!(notes[0].kind, SystemNoteKind::Command);
        assert_eq!(notes[0].title, "/commit -m x");
    }

    #[test]
    fn task_notification_title_uses_summary() {
        let text = "<task-notification>\n<task-id>x</task-id>\n<summary>Agent finished</summary>\n</task-notification>";
        let (_, notes) = extract_pseudo_tags(text);
        assert_eq!(notes[0].kind, SystemNoteKind::TaskNotification);
        assert_eq!(notes[0].title, "Task: Agent finished");
    }

    #[test]
    fn unterminated_tag_is_left_alone() {
        let (clean, notes) = extract_pseudo_tags("<system-reminder> oops");
        assert_eq!(clean, "<system-reminder> oops");
        assert!(notes.is_empty());
    }

    #[test]
    fn u64_of_rejects_garbage_numbers() {
        let v = serde_json::json!({ "a": 1e300, "b": -5.0, "c": 7.0, "d": 18446744073709551615u64, "e": "9" });
        assert_eq!(u64_of(&v, "a"), None);
        assert_eq!(u64_of(&v, "b"), None);
        assert_eq!(u64_of(&v, "c"), Some(7));
        assert_eq!(u64_of(&v, "d"), Some(u64::MAX));
        assert_eq!(u64_of(&v, "e"), None);
        let big = clip_input(serde_json::json!({ "x": "y".repeat(TOOL_INPUT_CAP) }));
        assert_eq!(big["_truncated"], true);
        assert_eq!(clip_input(serde_json::json!({ "x": 1 }))["x"], 1);
    }

    #[test]
    fn cap_text_cuts_on_char_boundary() {
        let s = "é".repeat(TOOL_TEXT_CAP); // 2 bytes each
        let (out, cut) = cap_text(&s);
        assert!(cut);
        assert!(out.len() <= TOOL_TEXT_CAP + 20);
        assert!(out.ends_with("[truncated]"));
        assert_eq!(cap_text("ok"), ("ok".to_string(), false));
    }

    #[test]
    fn structured_patch_renders_unified() {
        let p = serde_json::json!([{ "oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 2, "lines": ["-a", "+b", "+c"] }]);
        let d = structured_patch_to_unified(&p, Some("x.rs")).unwrap();
        assert!(d.starts_with("--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,2 @@\n-a\n+b\n+c\n"));
        assert!(structured_patch_to_unified(&serde_json::json!([]), None).is_none());
    }

    #[test]
    fn pr_urls_are_found_and_labelled() {
        let t = "Opened https://github.com/o/r/pull/12. Also (https://bitbucket.org/w/r/pull-requests/7) and https://github.com/o/r/issues/3";
        let urls = pr_urls(t);
        assert_eq!(urls, vec!["https://github.com/o/r/pull/12", "https://bitbucket.org/w/r/pull-requests/7"]);
        assert_eq!(pr_label(&urls[0]), "o/r#12");
    }
}
