//! Memory **inspector** for personal agents: what the agent has learned, item
//! by item, with where it learned it.
//!
//! The agent's durable memory stays the one Markdown file every session of the
//! agent reads and updates (`memory/notes.md` — runs, chat, any future channel
//! conversation with the agent all share it). Each top-level bullet is one
//! memory item; agents are told to start a bullet with its source tag —
//! `[chat]`, `[slack]`, `[telegram]`, `[vault]`, `[run]` (or `[user]` for a
//! note the user typed) — so the inspector can show provenance. Untagged
//! bullets read as source `notes`.
//!
//! Edits are line-addressed AND content-checked: a request names the line and
//! the exact text it expects there, on top of the document's optimistic
//! version, so a concurrent agent rewrite can never make "forget" delete the
//! wrong item.

use otto_core::{Error, Result};
use serde::Serialize;

/// Source tags the agent is told to use (anything else reads as `notes`).
pub const SOURCES: &[&str] = &["chat", "slack", "telegram", "vault", "run", "user"];

/// One remembered item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MemoryItem {
    /// 0-based line in `memory/notes.md` (the edit address).
    pub line: usize,
    /// The bullet's text without the marker and source tag.
    pub text: String,
    /// `chat` | `slack` | `telegram` | `vault` | `run` | `user` | `notes`.
    pub source: String,
    /// The nearest heading above the item ("" at top level).
    pub section: String,
    /// The raw line as stored (what an edit must quote back).
    pub raw: String,
}

fn bullet_body(line: &str) -> Option<&str> {
    // Top-level bullets only (indented sub-bullets belong to their parent).
    line.strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .map(str::trim)
        .filter(|b| !b.is_empty())
}

/// Split `[source] text` → (source, text).
fn split_source(body: &str) -> (String, String) {
    if let Some(rest) = body.strip_prefix('[') {
        if let Some((tag, text)) = rest.split_once(']') {
            let tag = tag.trim().to_ascii_lowercase();
            if SOURCES.contains(&tag.as_str()) {
                return (tag, text.trim().to_string());
            }
        }
    }
    ("notes".into(), body.to_string())
}

/// Every memory item in the file, in file order.
pub fn parse_items(md: &str) -> Vec<MemoryItem> {
    let mut section = String::new();
    let mut out = Vec::new();
    for (i, line) in md.lines().enumerate() {
        if let Some(h) = line.strip_prefix('#') {
            section = h.trim_start_matches('#').trim().to_string();
            continue;
        }
        if let Some(body) = bullet_body(line) {
            let (source, text) = split_source(body);
            out.push(MemoryItem {
                line: i,
                text,
                source,
                section: section.clone(),
                raw: line.to_string(),
            });
        }
    }
    out
}

/// Replace (`Some`) or forget (`None`) the item at `line`, which must still be
/// exactly `expected_raw` and a bullet. Keeps the item's source tag on edit
/// unless the new text brings its own. Returns the new document.
pub fn edit_item(
    md: &str,
    line: usize,
    expected_raw: &str,
    new_text: Option<&str>,
) -> Result<String> {
    let lines: Vec<&str> = md.lines().collect();
    let current = lines
        .get(line)
        .ok_or_else(|| Error::Conflict("that memory item no longer exists — reload".into()))?;
    if *current != expected_raw || bullet_body(current).is_none() {
        return Err(Error::Conflict(
            "the agent changed this memory since you opened it — reload and try again".into(),
        ));
    }
    let replacement = match new_text.map(str::trim) {
        None => None,
        Some("") => return Err(Error::Invalid("memory text is empty — use Forget".into())),
        Some(t) if t.contains('\n') => {
            return Err(Error::Invalid("a memory item is one line".into()))
        }
        Some(t) => {
            let (old_source, _) = split_source(bullet_body(current).unwrap_or_default());
            let (new_source, _) = split_source(t);
            Some(if new_source != "notes" || old_source == "notes" {
                format!("- {t}")
            } else {
                format!("- [{old_source}] {t}")
            })
        }
    };
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (i, l) in lines.iter().enumerate() {
        if i == line {
            if let Some(r) = &replacement {
                out.push(r.clone());
            }
        } else {
            out.push((*l).to_string());
        }
    }
    let mut joined = out.join("\n");
    if md.ends_with('\n') {
        joined.push('\n');
    }
    Ok(joined)
}

/// The instruction every session of the agent gets about tagging memories.
pub const TAGGING_INSTRUCTION: &str = "Write each memory as ONE top-level bullet that starts with \
where you learned it: `- [run] …` (an automated run), `- [chat] …` (a chat with the user), \
`- [slack] …` / `- [telegram] …` (a channel conversation), `- [vault] …` (a document). The user \
reviews, edits and deletes these items in the Memory inspector.";

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "# Scout — memory\n\nIntro paragraph.\n\n## People\n- [slack] Dana owns billing\n- Prefers short summaries\n  - nested detail\n* [run] CI flaky on mac\n- [bogus] tag kept as text\n";

    #[test]
    fn parses_items_with_sources_and_sections() {
        let items = parse_items(DOC);
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].source, "slack");
        assert_eq!(items[0].text, "Dana owns billing");
        assert_eq!(items[0].section, "People");
        assert_eq!(items[1].source, "notes");
        assert_eq!(items[2].source, "run");
        assert_eq!(items[2].raw, "* [run] CI flaky on mac");
        assert_eq!(items[3].source, "notes");
        assert_eq!(items[3].text, "[bogus] tag kept as text");
    }

    #[test]
    fn forget_and_edit_are_content_checked() {
        let items = parse_items(DOC);
        let dana = &items[0];
        let forgotten = edit_item(DOC, dana.line, &dana.raw, None).unwrap();
        assert!(!forgotten.contains("Dana"));
        assert!(forgotten.ends_with('\n'));
        assert_eq!(parse_items(&forgotten).len(), 3);
        // Edit keeps the source tag.
        let edited = edit_item(DOC, dana.line, &dana.raw, Some("Dana owns payments")).unwrap();
        assert!(edited.contains("- [slack] Dana owns payments"));
        // A stale expectation is a conflict, never a wrong delete.
        assert!(matches!(
            edit_item(DOC, dana.line, "- [slack] something else", None),
            Err(Error::Conflict(_))
        ));
        // Non-bullet lines are not items.
        assert!(edit_item(DOC, 2, "Intro paragraph.", None).is_err());
        assert!(edit_item(DOC, 99, "x", None).is_err());
        assert!(edit_item(DOC, dana.line, &dana.raw, Some("  ")).is_err());
        assert!(edit_item(DOC, dana.line, &dana.raw, Some("a\nb")).is_err());
    }
}
